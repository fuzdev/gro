import { test, assert, vi, onTestFinished, afterEach, beforeEach } from 'vitest';
import { wait } from '@fuzdev/fuz_util/async.ts';

import { gro_plugin_gen, type GroPluginGenOptions } from '#lib/gro_plugin_gen.ts';
import type { FileChange, ModuleGraph } from '#lib/module_graph.ts';
import type { PluginContext } from '#lib/plugin.ts';
import { SpawnError } from '#lib/spawn_error.ts';
import { is_task_error, SilentError } from '#lib/task.ts';
import { find_genfiles, is_gen_path } from '#lib/gen.ts';
import { load_module } from '#lib/modules.ts';

import { get_mock_spawn } from './spawn_mocks.ts';
import { create_mock_logger } from './test_helpers.ts';

vi.mock('#lib/spawn.ts', async () => (await import('./spawn_mocks.ts')).create_mock_spawn_module());

vi.mock('#lib/gen.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/gen.ts')>()),
	find_genfiles: vi.fn()
}));

vi.mock('#lib/modules.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/modules.ts')>()),
	load_module: vi.fn()
}));

const GENFILE = '/p/src/b.gen.ts';

beforeEach(() => {
	// a genfile with no `inputs`, so only the graph decides
	vi.mocked(load_module).mockResolvedValue({
		ok: true,
		id: GENFILE,
		mod: { gen: () => 'content' }
	});
});

afterEach(async () => {
	(await get_mock_spawn()).reset();
});

/**
 * A graph holding one genfile whose `dependents` query fails with `error` when there is one,
 * and on which every query rejects once closed, as on a closed graph,
 * holding the gen plugin's watch callback.
 */
const create_graph = (error?: Error) => {
	let closed = false;
	// returns the plugin's async callback's promise, which the graph never awaits
	let on_change: ((change: FileChange) => unknown) | undefined;
	const if_open = <T>(value: () => Promise<T>): Promise<T> =>
		closed ? Promise.reject(Error('ModuleGraph is closed')) : value();
	const graph: ModuleGraph = {
		modules: () => if_open(() => Promise.resolve(['/p/src/b.gen.ts'])),
		dependents: () => if_open(() => (error ? Promise.reject(error) : Promise.resolve(new Set()))),
		watch: (cb) => {
			on_change = cb;
			return Promise.resolve(() => Promise.resolve());
		},
		close: () => {
			closed = true;
			return Promise.resolve();
		}
	};
	const change = (id = '/p/src/a.ts', type: FileChange['type'] = 'update'): Promise<void> =>
		on_change!({ type, id }) as Promise<void>;
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
	const { graph, change } = create_graph();
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

test('a change re-invokes `gro gen` for the affected genfile with the whole environment', async () => {
	const mock = await get_mock_spawn();
	const { graph, change } = create_graph();
	await setup(graph, { flush_debounce_delay: 0 });
	await change(GENFILE);
	await vi.waitFor(() => assert.equal(mock.calls.length, 1));
	const [call] = mock.calls;
	assert.deepEqual(
		[call!.method, call!.name, call!.args, call!.program_options],
		['run', 'node_modules/.bin/gro', ['gen', GENFILE], { pass_env: 'all' }]
	);
});

test('a genfile deleted after being queued is dropped, and an emptied queue runs no gen', async () => {
	const mock = await get_mock_spawn();
	const { graph, change } = create_graph();
	await setup(graph, { flush_debounce_delay: 10 });
	await change(GENFILE);
	await change(GENFILE, 'delete');
	await wait(50);
	assert.equal(mock.calls.length, 0);
});

test('a deleted genfile leaves the rest of the queue to run', async () => {
	const mock = await get_mock_spawn();
	const other = '/p/src/c.gen.ts';
	const { graph, change } = create_graph();
	graph.modules = () => Promise.resolve([GENFILE, other]);
	await setup(graph, { flush_debounce_delay: 10 });
	await change(GENFILE);
	await change(other);
	await change(GENFILE, 'delete');
	await vi.waitFor(() => assert.equal(mock.calls.length, 1));
	assert.deepEqual(mock.calls[0]!.args, ['gen', other]);
});

test('a change queries the dependents once and runs only the genfiles among them', async () => {
	const mock = await get_mock_spawn();
	const dependent = '/p/src/c.gen.ts';
	const { graph, change } = create_graph();
	graph.modules = () => Promise.resolve([GENFILE, dependent]);
	const dependents = vi.fn<ModuleGraph['dependents']>(() => Promise.resolve(new Set([dependent])));
	graph.dependents = dependents;
	await setup(graph, { flush_debounce_delay: 0 });
	await change();
	await vi.waitFor(() => assert.equal(mock.calls.length, 1));
	assert.deepEqual(mock.calls[0]!.args, ['gen', dependent]);
	assert.equal(dependents.mock.calls.length, 1);
	assert.equal(dependents.mock.calls[0]![0], '/p/src/a.ts');
	assert.strictEqual(dependents.mock.calls[0]![1], is_gen_path);
});

test('invalid inputs are logged once per genfile across changes, naming each', async () => {
	const other = '/p/src/c.gen.ts';
	const { graph, change } = create_graph();
	graph.modules = () => Promise.resolve([GENFILE, other]);
	vi.mocked(load_module).mockResolvedValue({
		ok: true,
		id: GENFILE,
		mod: { gen: { generate: () => 'content', inputs: { patterns: ['./src/*.json'] } } }
	});
	const log = await setup(graph, { flush_debounce_delay: 0 });
	await change('/p/src/a.ts');
	await change('/p/src/d.ts');
	await change('/p/src/a.ts');
	const messages = vi.mocked(log.error).mock.calls.map(([message]) => String(message));
	assert.equal(messages.length, 2);
	assert.ok(messages.every((m) => m.includes('write `/src/*.json`')));
	assert.include(messages[0]!, 'b.gen.ts');
	assert.include(messages[1]!, 'c.gen.ts');
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

	const { graph, change } = create_graph();
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

test('a gen run that exits non-zero with watch logs nothing fatal and the queue keeps working', async () => {
	const mock = await get_mock_spawn();
	mock.result = { kind: 'exited', code: 1 };
	const { graph, change } = create_graph();
	await setup(graph, { flush_debounce_delay: 0 });
	await change(GENFILE);
	await vi.waitFor(() => assert.equal(mock.calls.length, 1));
	await change(GENFILE);
	await vi.waitFor(() => assert.equal(mock.calls.length, 2));
});

const setup_no_watch = async (
	genfiles: Array<string> = [GENFILE],
	invoke_task: PluginContext['invoke_task'] = vi.fn()
): Promise<unknown> => {
	vi.mocked(find_genfiles).mockResolvedValue({
		ok: true,
		value: { resolved_input_files: genfiles }
	} as never);
	const { graph } = create_graph();
	onTestFinished(() => graph.close());
	try {
		await gro_plugin_gen().setup!({
			dev: true,
			watch: false,
			log: create_mock_logger(),
			config: {},
			graph,
			invoke_task
		} as unknown as PluginContext);
	} catch (error) {
		return error;
	}
	return undefined;
};

test('with no watch, gen runs in-process, spawning no `gro`', async () => {
	const mock = await get_mock_spawn();
	mock.available = () => false; // none on `PATH` or in `.bin`, as with an absolute-path gro
	const invoke_task = vi.fn();
	assert.equal(await setup_no_watch(undefined, invoke_task as never), undefined);
	assert.deepEqual(invoke_task.mock.calls, [['gen']]);
	assert.equal(mock.calls.length, 0);
});

test('with no watch and no genfiles, gen does not run', async () => {
	const invoke_task = vi.fn();
	assert.equal(await setup_no_watch([], invoke_task as never), undefined);
	assert.equal(invoke_task.mock.calls.length, 0);
});

test('with no watch, a failing gen fails the setup with its error', async () => {
	const failure = new SilentError();
	const error = await setup_no_watch(undefined, () => Promise.reject(failure));
	assert.strictEqual(error, failure);
});
