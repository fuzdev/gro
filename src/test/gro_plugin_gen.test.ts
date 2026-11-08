import { test, assert, vi, onTestFinished, afterEach } from 'vitest';
import { wait } from '@fuzdev/fuz_util/async.ts';

import { gro_plugin_gen, type GroPluginGenOptions } from '#lib/gro_plugin_gen.ts';
import type { FileChange, ModuleGraph } from '#lib/module_graph.ts';
import type { PluginContext } from '#lib/plugin.ts';
import { SpawnError } from '#lib/spawn_error.ts';
import { is_task_error } from '#lib/task.ts';

import { get_mock_spawn } from './spawn_mocks.ts';
import { create_mock_logger } from './test_helpers.ts';

vi.mock('#lib/spawn.ts', async () => (await import('./spawn_mocks.ts')).create_mock_spawn_module());

afterEach(async () => {
	(await get_mock_spawn()).reset();
});

/**
 * A graph holding one genfile whose `dependents` query fails with `error`,
 * and on which every query rejects once closed, as on a closed graph,
 * holding the gen plugin's watch callback.
 */
const create_graph = (error: Error) => {
	let closed = false;
	// returns the plugin's async callback's promise, which the graph never awaits
	let on_change: ((change: FileChange) => unknown) | undefined;
	const if_open = <T>(value: () => Promise<T>): Promise<T> =>
		closed ? Promise.reject(Error('ModuleGraph is closed')) : value();
	const graph: ModuleGraph = {
		modules: () => if_open(() => Promise.resolve(['/p/src/b.gen.ts'])),
		dependents: () => if_open(() => Promise.reject(error)),
		watch: (cb) => {
			on_change = cb;
			return Promise.resolve(() => Promise.resolve());
		},
		close: () => {
			closed = true;
			return Promise.resolve();
		}
	};
	const change = (id = '/p/src/a.ts'): Promise<void> =>
		on_change!({ type: 'update', id }) as Promise<void>;
	return { graph, change };
};

const setup = async (graph: ModuleGraph, options?: GroPluginGenOptions) => {
	onTestFinished(() => graph.close());
	const log = create_mock_logger();
	await gro_plugin_gen(options).setup!({
		dev: true,
		watch: true,
		log,
		graph
	} as unknown as PluginContext);
	return log;
};

test('a change checked after the graph closed ends quietly', async () => {
	const { graph, change } = create_graph(Error('unused'));
	const log = await setup(graph);
	await graph.close();
	await change(); // resolves, so no unhandled rejection
	assert.equal(vi.mocked(log.error).mock.calls.length, 0);
});

test('a change whose check fails on an open graph logs the error', async () => {
	const error = Error('boom');
	const { graph, change } = create_graph(error);
	const log = await setup(graph);
	await change();
	assert.deepEqual(vi.mocked(log.error).mock.calls, [
		['[gen] failed to check for genfiles to run for', '/p/src/a.ts', error]
	]);
});

const GENFILE = '/p/src/b.gen.ts';

test('a change re-invokes `gro gen` for the affected genfile with the whole environment', async () => {
	const mock = await get_mock_spawn();
	const { graph, change } = create_graph(Error('unused'));
	await setup(graph, { flush_debounce_delay: 0 });
	await change(GENFILE);
	await vi.waitFor(() => assert.equal(mock.calls.length, 1));
	const [call] = mock.calls;
	assert.deepEqual(
		[call!.method, call!.name, call!.args, call!.program_options],
		['run', 'node_modules/.bin/gro', ['gen', GENFILE], { pass_env: 'all' }]
	);
});

test('a gen run that throws logs the error, and the queue keeps working', async () => {
	const mock = await get_mock_spawn();
	const spawn_error = new SpawnError(
		'not_executable',
		'node_modules/.bin/gro',
		"not_executable: node_modules/.bin/gro isn't executable"
	);
	mock.find_error = () => spawn_error;
	const on_unhandled = vi.fn();
	process.on('unhandledRejection', on_unhandled);
	onTestFinished(() => void process.off('unhandledRejection', on_unhandled));

	const { graph, change } = create_graph(Error('unused'));
	const log = await setup(graph, { flush_debounce_delay: 0 });
	await change(GENFILE);
	await vi.waitFor(() => assert.equal(vi.mocked(log.error).mock.calls.length, 1));
	await wait(0); // an unhandled rejection is reported after the microtask queue drains
	assert.equal(on_unhandled.mock.calls.length, 0);
	const [label, logged] = vi.mocked(log.error).mock.calls[0]!;
	assert.equal(label, '[gen] failed to run gen');
	assert.ok(is_task_error(logged));
	assert.strictEqual(logged.cause, spawn_error);

	mock.find_error = () => undefined;
	await change(GENFILE);
	await vi.waitFor(() => assert.equal(mock.calls.length, 1));
	assert.deepEqual(mock.calls[0]!.args, ['gen', GENFILE]);
	assert.equal(vi.mocked(log.error).mock.calls.length, 1);
});
