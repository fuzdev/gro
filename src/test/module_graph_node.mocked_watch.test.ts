import { test, assert, vi, onTestFinished } from 'vitest';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { create_node_module_graph, create_node_watcher } from '#lib/module_graph_node.ts';
import type { FileChange, ModuleGraph, Watcher } from '#lib/module_graph.ts';
import type { WatchDirOptions } from '#lib/internal/watch_dir.ts';

import { create_mock_logger } from './test_helpers.ts';

// the options of each `watch_dir` call, to emit changes and errors by hand
const watches: Array<WatchDirOptions> = [];

vi.mock('#lib/internal/watch_dir.ts', () => ({
	watch_dir: (options: WatchDirOptions) => {
		watches.push(options);
		return { init: async () => {}, close: async () => {} };
	}
}));

const create_dir = async (): Promise<string> => {
	const dir = await mkdtemp(join(tmpdir(), 'gro_module_graph_mocked_'));
	onTestFinished(async () => {
		watches.length = 0;
		await rm(dir, { recursive: true, force: true });
	});
	return dir;
};

// closes `closable` when the test finishes, passed or failed
const closing = <T extends ModuleGraph | Watcher>(closable: T): T => {
	onTestFinished(() => closable.close());
	return closable;
};

const settle = (): Promise<void> => new Promise((r) => setTimeout(r, 20));

test('the graph forwards a change whose read finds no file as one delete', async () => {
	const dir = await create_dir();
	const id = join(dir, 'a.ts');
	await writeFile(id, 'export const a = 1;\n');
	const graph = closing(create_node_module_graph({ watch: { dir, filter: null } }));
	const changes: Array<FileChange> = [];
	await graph.watch((change) => changes.push(change));
	const [options] = watches;
	assert.ok(options);
	options.on_change({ type: 'add', path: id, is_directory: false });
	await settle();
	await rm(id);
	// a save racing a delete: chokidar reports the change, the read finds nothing,
	// then reports the unlink, which is no second deletion
	options.on_change({ type: 'update', path: id, is_directory: false });
	await settle();
	options.on_change({ type: 'delete', path: id, is_directory: false });
	await settle();
	assert.deepEqual(changes, [
		{ type: 'add', id },
		{ type: 'delete', id }
	]);
});

test('the graph and the watcher warn of a watch error, which is not fatal', async () => {
	const dir = await create_dir();
	const graph_log = create_mock_logger();
	const watcher_log = create_mock_logger();
	const graph = closing(create_node_module_graph({ watch: { dir, filter: null }, log: graph_log }));
	const watcher = closing(create_node_watcher({ watch: { dir, filter: null }, log: watcher_log }));
	await graph.modules();
	await watcher.watch(() => {});
	assert.equal(watches.length, 2);
	const error = Object.assign(Error('denied'), { code: 'EACCES' });
	for (const options of watches) {
		assert.ok(options.on_error);
		options.on_error(error);
	}
	for (const log of [graph_log, watcher_log]) {
		assert.deepEqual(vi.mocked(log.warn).mock.calls, [
			['watch error, changes may be missed:', error]
		]);
		assert.equal(vi.mocked(log.error).mock.calls.length, 0);
	}
});

test('with no log, a watch error warns through `console`', async () => {
	const dir = await create_dir();
	const console_warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
	onTestFinished(() => console_warn.mockRestore());
	const graph = closing(create_node_module_graph({ watch: { dir, filter: null } }));
	const watcher = closing(create_node_watcher({ watch: { dir, filter: null } }));
	await graph.modules();
	await watcher.watch(() => {});
	const error = Error('denied');
	for (const options of watches) options.on_error!(error);
	assert.deepEqual(console_warn.mock.calls, [
		['watch error, changes may be missed:', error],
		['watch error, changes may be missed:', error]
	]);
});
