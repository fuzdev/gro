import { test, assert, vi, onTestFinished } from 'vitest';

import { gro_plugin_gen } from '#lib/gro_plugin_gen.ts';
import type { FileChange, ModuleGraph } from '#lib/module_graph.ts';
import type { PluginContext } from '#lib/plugin.ts';

import { create_mock_logger } from './test_helpers.ts';

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
	const change = (): Promise<void> =>
		on_change!({ type: 'update', id: '/p/src/a.ts' }) as Promise<void>;
	return { graph, change };
};

const setup = async (graph: ModuleGraph) => {
	onTestFinished(() => graph.close());
	const log = create_mock_logger();
	await gro_plugin_gen().setup!({ dev: true, watch: true, log, graph } as unknown as PluginContext);
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
