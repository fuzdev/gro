import { describe, test, assert, onTestFinished } from 'vitest';
import { join } from 'node:path';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';

import { create_node_module_graph, create_node_watcher } from '#lib/module_graph_node.ts';
import type { FileChange } from '#lib/module_graph.ts';

const WAIT_TIMEOUT = 5000;

/**
 * Creates a temp project: `src/` holding `a.ts` <- `b.ts` <- `c.gen.ts`,
 * `b.ts` also importing `outside.ts` beside `src/`, and an unrelated `d.ts`
 * importing `missing.ts`, which doesn't exist.
 */
const create_project = async (): Promise<{ root: string; dir: string }> => {
	const root = await mkdtemp(join(tmpdir(), 'gro_module_graph_'));
	onTestFinished(() => rm(root, { recursive: true, force: true }));
	const dir = join(root, 'src');
	await mkdir(dir);
	await writeFile(join(root, 'outside.ts'), 'export const o = 1;\n');
	await writeFile(join(dir, 'a.ts'), 'export const a = 1;\n');
	await writeFile(
		join(dir, 'b.ts'),
		"import {a} from './a.ts';\nimport {o} from '../outside.ts';\nexport const b = a + o;\n"
	);
	await writeFile(
		join(dir, 'c.gen.ts'),
		"import {b} from './b.ts';\nexport const gen = () => b;\n"
	);
	await writeFile(join(dir, 'd.ts'), "import './missing.ts';\nexport const d = 1;\n");
	return { root, dir };
};

/**
 * Collects changes and resolves `wait_for` once a matching change arrives.
 */
const create_change_log = () => {
	const changes: Array<FileChange> = [];
	const waiting: Array<{ match: (c: FileChange) => boolean; resolve: () => void }> = [];
	return {
		changes,
		on_change: (change: FileChange): void => {
			changes.push(change);
			for (const w of waiting.slice()) {
				if (w.match(change)) {
					waiting.splice(waiting.indexOf(w), 1);
					w.resolve();
				}
			}
		},
		wait_for: (match: (c: FileChange) => boolean): Promise<void> => {
			if (changes.some(match)) return Promise.resolve();
			return new Promise((resolve, reject) => {
				const timeout = setTimeout(
					() => reject(Error('timed out waiting for a change')),
					WAIT_TIMEOUT
				);
				waiting.push({
					match,
					resolve: () => {
						clearTimeout(timeout);
						resolve();
					}
				});
			});
		}
	};
};

/**
 * Resolves after pending I/O callbacks, later than any microtask.
 */
const next_macrotask = (): Promise<'macrotask'> =>
	new Promise((resolve) => setImmediate(() => resolve('macrotask')));

const assert_rejects_closed = async (promise: Promise<unknown>, message: string): Promise<void> => {
	try {
		await promise;
	} catch (error) {
		assert(error instanceof Error);
		assert.equal(error.message, message);
		return;
	}
	assert.fail('expected a rejection');
};

describe('create_node_module_graph', () => {
	test('lists the project modules, filtered, excluding files outside the dir', async () => {
		const { dir } = await create_project();
		const graph = create_node_module_graph({ watch: { dir, filter: null } });
		onTestFinished(() => graph.close());

		const modules = await graph.modules();
		assert.sameMembers(
			modules,
			['a.ts', 'b.ts', 'c.gen.ts', 'd.ts'].map((f) => join(dir, f))
		);
		assert.notInclude(modules, join(dir, 'missing.ts'));
		assert.deepEqual(await graph.modules((id) => id.includes('.gen.')), [join(dir, 'c.gen.ts')]);
	});

	test('the watch filter excludes paths from the project modules', async () => {
		const { dir } = await create_project();
		const graph = create_node_module_graph({
			watch: { dir, filter: (path) => !path.endsWith('d.ts') }
		});
		onTestFinished(() => graph.close());

		assert.notInclude(await graph.modules(), join(dir, 'd.ts'));
	});

	test('finds transitive dependents, the filter narrowing results and not traversal', async () => {
		const { root, dir } = await create_project();
		const graph = create_node_module_graph({ watch: { dir, filter: null } });
		onTestFinished(() => graph.close());

		assert.sameMembers(Array.from(await graph.dependents(join(dir, 'a.ts'))), [
			join(dir, 'b.ts'),
			join(dir, 'c.gen.ts')
		]);
		// `b.ts` fails the filter, yet the traversal goes through it to `c.gen.ts`
		assert.deepEqual(
			Array.from(await graph.dependents(join(dir, 'a.ts'), (id) => id.includes('.gen.'))),
			[join(dir, 'c.gen.ts')]
		);
		// a file outside the dir is in the graph when a project module imports it
		assert.sameMembers(Array.from(await graph.dependents(join(root, 'outside.ts'))), [
			join(dir, 'b.ts'),
			join(dir, 'c.gen.ts')
		]);
		assert.equal((await graph.dependents(join(dir, 'd.ts'))).size, 0);
		assert.equal((await graph.dependents(join(dir, 'nowhere.ts'))).size, 0);
		// an imported path that doesn't exist yet has its importers as dependents
		assert.include(await graph.dependents(join(dir, 'missing.ts')), join(dir, 'd.ts'));
	});

	test('emits an add per project module, then content changes', async () => {
		const { root, dir } = await create_project();
		const graph = create_node_module_graph({ watch: { dir, filter: null } });
		onTestFinished(() => graph.close());

		const log = create_change_log();
		const unwatch = await graph.watch(log.on_change);
		onTestFinished(unwatch);

		const initial = log.changes.slice();
		assert.ok(initial.every((c) => c.type === 'add'));
		assert.sameMembers(
			initial.map((c) => c.id),
			['a.ts', 'b.ts', 'c.gen.ts', 'd.ts'].map((f) => join(dir, f))
		);
		assert.notInclude(
			initial.map((c) => c.id),
			join(root, 'outside.ts')
		);
		assert.notInclude(
			initial.map((c) => c.id),
			join(dir, 'missing.ts')
		);

		await writeFile(join(dir, 'a.ts'), 'export const a = 2;\n');
		await log.wait_for((c) => c.type === 'update' && c.id === join(dir, 'a.ts'));

		await writeFile(join(dir, 'e.ts'), 'export const e = 1;\n');
		await log.wait_for((c) => c.type === 'add' && c.id === join(dir, 'e.ts'));
		assert.include(await graph.modules(), join(dir, 'e.ts'));

		// an imported file that didn't exist emits an `add` once created
		await writeFile(join(dir, 'missing.ts'), 'export const m = 1;\n');
		await log.wait_for((c) => c.type === 'add' && c.id === join(dir, 'missing.ts'));
		assert.include(await graph.modules(), join(dir, 'missing.ts'));
	});

	test('closing a graph that never loaded resolves', async () => {
		const graph = create_node_module_graph({ watch: { dir: '/nonexistent', filter: null } });
		await graph.close();
	});

	test('closing is permanent, so later queries and watches reject', async () => {
		const { dir } = await create_project();
		const graph = create_node_module_graph({ watch: { dir, filter: null } });
		await graph.modules();
		await graph.close();

		// rejects before any I/O, not after loading a fresh graph
		const modules = graph.modules();
		const first = await Promise.race([
			modules.then(
				() => 'resolved',
				() => 'rejected'
			),
			next_macrotask()
		]);
		assert.equal(first, 'rejected');
		await assert_rejects_closed(modules, 'ModuleGraph is closed');
		await assert_rejects_closed(graph.dependents(join(dir, 'a.ts')), 'ModuleGraph is closed');
		await assert_rejects_closed(
			graph.watch(() => {}),
			'ModuleGraph is closed'
		);
	});

	test('a query pending when the graph closes rejects', async () => {
		const { dir } = await create_project();
		const graph = create_node_module_graph({ watch: { dir, filter: null } });
		const modules = graph.modules();
		const watching = graph.watch(() => {});
		await graph.close();

		await assert_rejects_closed(modules, 'ModuleGraph is closed');
		await assert_rejects_closed(watching, 'ModuleGraph is closed');
	});

	test('a watch the graph closes during while subscribing rejects', async () => {
		const { dir } = await create_project();
		const graph = create_node_module_graph({ watch: { dir, filter: null } });
		await graph.modules();
		// closes while the Filer delivers the initial adds, after loading
		let closing: Promise<void> | undefined;
		const watching = graph.watch(() => {
			closing ??= graph.close();
		});

		await assert_rejects_closed(watching, 'ModuleGraph is closed');
		assert.ok(closing);
		await closing;
	});

	test('a second close resolves no earlier than the first teardown', async () => {
		const { dir } = await create_project();
		const graph = create_node_module_graph({ watch: { dir, filter: null } });
		await graph.modules();
		let first_closed = false;
		const first = graph.close().then(() => {
			first_closed = true;
		});
		await graph.close();
		assert.ok(first_closed);
		await first;
	});
});

describe('create_node_watcher', () => {
	test('reports changes from now on, not the existing files', async () => {
		const { dir } = await create_project();
		const watcher = create_node_watcher({ watch: { dir, filter: null } });
		onTestFinished(() => watcher.close());

		const log = create_change_log();
		await watcher.watch(log.on_change);

		await writeFile(join(dir, 'e.ts'), 'export const e = 1;\n');
		await log.wait_for((c) => c.type === 'add' && c.id === join(dir, 'e.ts'));
		assert.notOk(log.changes.some((c) => c.id === join(dir, 'a.ts')));

		// directories emit nothing, waiting on a file inside so the dir's event came first
		await mkdir(join(dir, 'sub'));
		await writeFile(join(dir, 'sub', 'f.ts'), 'export const f = 1;\n');
		await log.wait_for((c) => c.type === 'add' && c.id === join(dir, 'sub', 'f.ts'));
		assert.notOk(log.changes.some((c) => c.id === join(dir, 'sub')));

		await rm(join(dir, 'e.ts'));
		await log.wait_for((c) => c.type === 'delete' && c.id === join(dir, 'e.ts'));
	});

	test('the returned function stops the watch', async () => {
		const { dir } = await create_project();
		const watcher = create_node_watcher({ watch: { dir, filter: null } });
		onTestFinished(() => watcher.close());

		const log = create_change_log();
		const unwatch = await watcher.watch(log.on_change);
		await unwatch();
		await writeFile(join(dir, 'e.ts'), 'export const e = 1;\n');

		// a second watch sees the next change, the stopped one never does
		const log2 = create_change_log();
		await watcher.watch(log2.on_change);
		await writeFile(join(dir, 'f.ts'), 'export const f = 1;\n');
		await log2.wait_for((c) => c.id === join(dir, 'f.ts'));
		assert.equal(log.changes.length, 0);
	});

	test('the filter excludes paths', async () => {
		const { dir } = await create_project();
		const watcher = create_node_watcher({ watch: { dir, filter: (p) => !p.endsWith('x.ts') } });
		onTestFinished(() => watcher.close());

		const log = create_change_log();
		await watcher.watch(log.on_change);

		await writeFile(join(dir, 'x.ts'), 'export const x = 1;\n');
		await writeFile(join(dir, 'y.ts'), 'export const y = 1;\n');
		await log.wait_for((c) => c.id === join(dir, 'y.ts'));
		assert.notOk(log.changes.some((c) => c.id === join(dir, 'x.ts')));
	});

	test('closing is permanent, so later watches reject', async () => {
		const { dir } = await create_project();
		const watcher = create_node_watcher({ watch: { dir, filter: null } });
		await watcher.close();

		await assert_rejects_closed(
			watcher.watch(() => {}),
			'Watcher is closed'
		);
	});

	test('a watch pending when the watcher closes rejects', async () => {
		const { dir } = await create_project();
		const watcher = create_node_watcher({ watch: { dir, filter: null } });
		const watching = watcher.watch(() => {});
		await watcher.close();

		await assert_rejects_closed(watching, 'Watcher is closed');
	});

	test('a watch closed while chokidar starts closes once ready and rejects', async () => {
		const { dir } = await create_project();
		const watcher = create_node_watcher({ watch: { dir, filter: null } });
		// caches the module import so the next watch gets to start chokidar
		await (await watcher.watch(() => {}))();
		const watching = watcher.watch(() => {});
		let watch_settled = false;
		const settle = () => {
			watch_settled = true;
		};
		watching.then(settle, settle);
		await next_macrotask();
		await watcher.close();
		// `close` waits for the starting watch to close itself
		assert.ok(watch_settled);

		await assert_rejects_closed(watching, 'Watcher is closed');
	});

	test('a second close resolves no earlier than the first teardown', async () => {
		const { dir } = await create_project();
		const watcher = create_node_watcher({ watch: { dir, filter: null } });
		await watcher.watch(() => {});
		let first_closed = false;
		const first = watcher.close().then(() => {
			first_closed = true;
		});
		await watcher.close();
		assert.ok(first_closed);
		await first;
	});
});
