import { test, assert, vi } from 'vitest';
import { join, resolve } from 'node:path';
import { mkdir, mkdtemp, realpath, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';

import type { WatchNodeFs, WatcherChange } from '#lib/internal/watch_dir.ts';
import { Filer, filter_dependents } from '#lib/internal/filer.ts';
import type { Disknode } from '#lib/internal/disknode.ts';

const fixtures_dir = resolve(import.meta.dirname, '../fixtures');

const create_test_disknode = (id: string, contents: string | null = null): Disknode => ({
	id,
	contents,
	external: false,
	ctime: 1,
	mtime: 1,
	content_hash: null,
	dependents: new Map(),
	dependencies: new Map()
});

// filter_dependents tests
test('filter_dependents finds direct dependents', () => {
	// Create a simple dependency graph: A <- B <- C
	const fileA = create_test_disknode('/test/a.ts', 'export const a = 1');
	const fileB = create_test_disknode('/test/b.ts', "import {a} from './a'");
	const fileC = create_test_disknode('/test/c.ts', "import {b} from './b'");

	// Set up dependencies: A <- B <- C
	fileB.dependencies.set(fileA.id, fileA);
	fileA.dependents.set(fileB.id, fileB);

	fileC.dependencies.set(fileB.id, fileB);
	fileB.dependents.set(fileC.id, fileC);

	const get_by_id = (id: string) => {
		if (id === fileA.id) return fileA;
		if (id === fileB.id) return fileB;
		if (id === fileC.id) return fileC;
		return undefined;
	};

	// Find all dependents of A
	const results = filter_dependents(fileA, get_by_id);

	assert.equal(results.size, 2);
	assert.ok(results.has(fileB.id));
	assert.ok(results.has(fileC.id));
});

test('filter_dependents with filter predicate', () => {
	// Create dependency graph: A <- B <- C
	const fileA = create_test_disknode('/test/a.ts', 'export const a = 1');
	const fileB = create_test_disknode('/test/b.js', "import {a} from './a'");
	const fileC = create_test_disknode('/test/c.ts', "import {b} from './b'");

	// Set up dependencies
	fileB.dependencies.set(fileA.id, fileA);
	fileA.dependents.set(fileB.id, fileB);

	fileC.dependencies.set(fileB.id, fileB);
	fileB.dependents.set(fileC.id, fileC);

	const get_by_id = (id: string) => {
		if (id === fileA.id) return fileA;
		if (id === fileB.id) return fileB;
		if (id === fileC.id) return fileC;
		return undefined;
	};

	// Find only .ts dependents of A
	const results = filter_dependents(fileA, get_by_id, (id) => id.endsWith('.ts'));

	// Should only include C (not B which is .js)
	assert.equal(results.size, 1);
	assert.ok(results.has(fileC.id));
	assert.ok(!results.has(fileB.id));
});

test('filter_dependents handles circular dependencies', () => {
	// Create circular dependency: A <- B <- C <- A
	const fileA = create_test_disknode('/test/a.ts', "import {c} from './c'");
	const fileB = create_test_disknode('/test/b.ts', "import {a} from './a'");
	const fileC = create_test_disknode('/test/c.ts', "import {b} from './b'");

	// Set up circular dependencies
	fileB.dependencies.set(fileA.id, fileA);
	fileA.dependents.set(fileB.id, fileB);

	fileC.dependencies.set(fileB.id, fileB);
	fileB.dependents.set(fileC.id, fileC);

	fileA.dependencies.set(fileC.id, fileC);
	fileC.dependents.set(fileA.id, fileA);

	const get_by_id = (id: string) => {
		if (id === fileA.id) return fileA;
		if (id === fileB.id) return fileB;
		if (id === fileC.id) return fileC;
		return undefined;
	};

	// Should handle circular deps without infinite loop
	const results = filter_dependents(fileA, get_by_id);

	// Should include all files in the cycle
	assert.equal(results.size, 3);
	assert.ok(results.has(fileA.id));
	assert.ok(results.has(fileB.id));
	assert.ok(results.has(fileC.id));
});

test('filter_dependents returns empty set when no dependents', () => {
	const fileA = create_test_disknode('/test/a.ts', 'export const a = 1');

	const get_by_id = (id: string) => (id === fileA.id ? fileA : undefined);

	const results = filter_dependents(fileA, get_by_id);

	assert.equal(results.size, 0);
});

// Dependency graph building and updates tests
test('builds bidirectional dependency links from parsed imports', async () => {
	const importer_path = resolve(fixtures_dir, 'filer_local_import.ts');
	const dep_path = resolve(fixtures_dir, 'filer_builtin_import.ts');

	const mock_watch_dir = vi.fn((options) => {
		const mock_watcher: WatchNodeFs = {
			init: vi.fn(async () => {
				options.on_change({ type: 'add', path: importer_path, is_directory: false });
			}),
			close: vi.fn(async () => {})
		};
		return mock_watcher;
	});

	const filer = new Filer({ watch_dir: mock_watch_dir });
	await filer.init();

	const importer = filer.get_by_id(importer_path);
	assert.ok(importer);
	assert.ok(importer.contents);

	// Importer should have the dependency in its dependencies map
	assert.ok(importer.dependencies.has(dep_path), 'importer should depend on dep');

	// Dep should have the importer in its dependents map
	const dep = filer.get_by_id(dep_path);
	assert.ok(dep);
	assert.ok(dep.dependents.has(importer_path), 'dep should have importer as dependent');
});

test('skips non-file schemes in transitive dependency resolution', async () => {
	const importer_path = resolve(fixtures_dir, 'filer_local_import.ts');
	const dep_path = resolve(fixtures_dir, 'filer_builtin_import.ts');

	const mock_watch_dir = vi.fn((options) => {
		const mock_watcher: WatchNodeFs = {
			init: vi.fn(async () => {
				options.on_change({ type: 'add', path: importer_path, is_directory: false });
			}),
			close: vi.fn(async () => {})
		};
		return mock_watcher;
	});

	const filer = new Filer({ watch_dir: mock_watch_dir });
	await filer.init();

	// Wait for deferred external file processing
	await new Promise((r) => setTimeout(r, 20));

	// The dependency (filer_builtin_import.ts) imports node:crypto,
	// which should not appear anywhere in the graph
	const dep = filer.get_by_id(dep_path);
	assert.ok(dep);
	assert.equal(dep.dependencies.size, 0, 'builtin-only file should have no tracked dependencies');

	// Verify no file in the graph has a path containing 'crypto'
	for (const [id] of filer.files) {
		assert.ok(!id.includes('crypto'), `unexpected crypto entry in graph: ${id}`);
	}
});

test('an import of a missing file links once the file is added', async () => {
	const dir = await mkdtemp(join(tmpdir(), 'gro_filer_missing_dep_'));
	try {
		const importer_path = join(dir, 'importer.ts');
		const dep_path = join(dir, 'dep.ts');
		await writeFile(importer_path, "import {dep} from './dep.ts';\nexport const a = dep;\n");

		let on_change_callback: ((change: any) => void) | null = null as any;
		const mock_watch_dir = vi.fn((options) => {
			on_change_callback = options.on_change;
			const mock_watcher: WatchNodeFs = {
				init: vi.fn(async () => {
					options.on_change({ type: 'add', path: importer_path, is_directory: false });
				}),
				close: vi.fn(async () => {})
			};
			return mock_watcher;
		});

		const filer = new Filer({ watch_dir: mock_watch_dir });
		await filer.init();

		// the edge is recorded to the exact named path before the file exists
		const importer = filer.get_by_id(importer_path);
		assert.ok(importer);
		assert.ok(importer.dependencies.has(dep_path), 'importer should depend on the missing dep');
		const dep_before = filer.get_by_id(dep_path);
		assert.ok(dep_before);
		assert.equal(dep_before.contents, null);

		await writeFile(dep_path, 'export const dep = 1;\n');
		assert.ok(on_change_callback);
		on_change_callback({ type: 'add', path: dep_path, is_directory: false });
		await new Promise((r) => setTimeout(r, 20));

		const dep = filer.get_by_id(dep_path);
		assert.ok(dep);
		assert.ok(dep.contents, 'dep contents should load once added');
		assert.ok(dep.dependents.has(importer_path), 'dep should have importer as dependent');
		assert.ok(filter_dependents(dep, filer.get_by_id).has(importer_path));

		await filer.close();
	} finally {
		await rm(dir, { recursive: true, force: true });
	}
});

test('a package file reached by a bare specifier keeps its edge but is never read, even when linked', async () => {
	const dir = await mkdtemp(join(tmpdir(), 'gro_filer_packages_'));
	try {
		const src = join(dir, 'proj/src');
		await mkdir(src, { recursive: true });
		await writeFile(
			join(dir, 'proj/package.json'),
			JSON.stringify({ name: 'proj', type: 'module' })
		);
		// an installed package
		const pkg = join(dir, 'proj/node_modules/pkg');
		await mkdir(pkg, { recursive: true });
		await writeFile(
			join(pkg, 'package.json'),
			JSON.stringify({ name: 'pkg', type: 'module', exports: './index.js' })
		);
		await writeFile(join(pkg, 'index.js'), "import './other.js';\nimport 'missing_peer';\n");
		// a linked package, which realpaths outside `node_modules`
		const sibling = join(dir, 'sibling');
		await mkdir(sibling);
		await writeFile(
			join(sibling, 'package.json'),
			JSON.stringify({ name: '@scope/x', type: 'module', exports: './index.js' })
		);
		await writeFile(join(sibling, 'index.js'), "import './util.js';\n");
		await mkdir(join(dir, 'proj/node_modules/@scope'));
		await symlink(sibling, join(dir, 'proj/node_modules/@scope/x'));
		// a project file outside `src/` through a subpath import is read
		await writeFile(
			join(dir, 'proj/package.json'),
			JSON.stringify({ name: 'proj', type: 'module', imports: { '#cfg': './cfg.ts' } })
		);
		await writeFile(join(dir, 'proj/cfg.ts'), "import './cfg_dep.ts';\n");
		const importer_path = join(src, 'a.ts');
		await writeFile(importer_path, "import 'pkg';\nimport '@scope/x';\nimport '#cfg';\n");

		const log = { error: vi.fn(), warn: vi.fn(), info: vi.fn(), debug: vi.fn() };
		const filer = new Filer({
			watch_dir: (options) => ({
				init: async () => {
					options.on_change({ type: 'add', path: importer_path, is_directory: false });
				},
				close: async () => {}
			}),
			watch_dir_options: { dir: src },
			log: log as any
		});
		await filer.init();
		await new Promise((r) => setTimeout(r, 20));

		const real_dir = await realpath(dir);
		for (const pkg_path of [
			join(real_dir, 'proj/node_modules/pkg/index.js'),
			join(real_dir, 'sibling/index.js')
		]) {
			const pkg_node = filer.get_by_id(pkg_path);
			assert.ok(pkg_node, `the edge to ${pkg_path} is recorded`);
			assert.ok(pkg_node.external);
			assert.ok(pkg_node.dependents.has(importer_path));
			assert.equal(pkg_node.contents, null, `${pkg_path} is never read`);
			assert.equal(pkg_node.dependencies.size, 0, `${pkg_path} is never parsed`);
		}
		assert.equal(filer.get_by_id(join(real_dir, 'proj/node_modules/pkg/other.js')), undefined);
		assert.equal(filer.get_by_id(join(real_dir, 'sibling/util.js')), undefined);
		const cfg = filer.get_by_id(join(real_dir, 'proj/cfg.ts'));
		assert.ok(cfg?.contents, 'a subpath import is read');
		assert.ok(cfg.dependencies.has(join(real_dir, 'proj/cfg_dep.ts')));
		assert.equal(log.error.mock.calls.length + log.debug.mock.calls.length, 0);

		await filer.close();
	} finally {
		await rm(dir, { recursive: true, force: true });
	}
});

test('a file reached by both a bare specifier and a path or subpath import is read in either order', async () => {
	for (const order of [
		['pkg', '#p'],
		['#p', 'pkg'],
		['pkg', '../node_modules/pkg/index.js'],
		['../node_modules/pkg/index.js', 'pkg']
	]) {
		const dir = await mkdtemp(join(tmpdir(), 'gro_filer_read_order_'));
		try {
			const src = join(dir, 'src');
			const pkg = join(dir, 'node_modules/pkg');
			await mkdir(src);
			await mkdir(pkg, { recursive: true });
			await writeFile(
				join(dir, 'package.json'),
				JSON.stringify({ name: 'proj', type: 'module', imports: { '#p': 'pkg' } })
			);
			await writeFile(
				join(pkg, 'package.json'),
				JSON.stringify({ name: 'pkg', type: 'module', exports: './index.js' })
			);
			await writeFile(join(pkg, 'index.js'), "import './other.js';\n");
			await writeFile(join(pkg, 'other.js'), 'export {};\n');
			const importer_path = join(src, 'a.ts');
			await writeFile(importer_path, order.map((sp) => `import '${sp}';`).join('\n'));
			const filer = new Filer({
				watch_dir: (options) => ({
					init: async () => {
						options.on_change({ type: 'add', path: importer_path, is_directory: false });
					},
					close: async () => {}
				}),
				watch_dir_options: { dir: src }
			});
			await filer.init();
			await new Promise((r) => setTimeout(r, 20));

			const real_pkg = join(await realpath(dir), 'node_modules/pkg');
			const pkg_node = filer.get_by_id(join(real_pkg, 'index.js'));
			assert.ok(pkg_node?.contents, `read for ${order.join(' then ')}`);
			assert.ok(pkg_node.dependencies.has(join(real_pkg, 'other.js')));

			await filer.close();
		} finally {
			await rm(dir, { recursive: true, force: true });
		}
	}
});

test('an edit adding a path or subpath import of a file a bare import reached reads it', async () => {
	for (const added of ['#p', '../node_modules/pkg/index.js']) {
		const dir = await mkdtemp(join(tmpdir(), 'gro_filer_read_on_update_'));
		try {
			const src = join(dir, 'src');
			const pkg = join(dir, 'node_modules/pkg');
			await mkdir(src);
			await mkdir(pkg, { recursive: true });
			await writeFile(
				join(dir, 'package.json'),
				JSON.stringify({ name: 'proj', type: 'module', imports: { '#p': 'pkg' } })
			);
			await writeFile(
				join(pkg, 'package.json'),
				JSON.stringify({ name: 'pkg', type: 'module', exports: './index.js' })
			);
			await writeFile(join(pkg, 'index.js'), "import './other.js';\n");
			await writeFile(join(pkg, 'other.js'), 'export {};\n');
			const importer_path = join(src, 'a.ts');
			await writeFile(importer_path, "import 'pkg';\n");
			let on_change: ((change: WatcherChange) => void) | undefined;
			const filer = new Filer({
				watch_dir: (options) => {
					on_change = options.on_change;
					return {
						init: async () => {
							options.on_change({ type: 'add', path: importer_path, is_directory: false });
						},
						close: async () => {}
					};
				},
				watch_dir_options: { dir: src }
			});
			await filer.init();
			await new Promise((r) => setTimeout(r, 20));

			const real_pkg = join(await realpath(dir), 'node_modules/pkg');
			assert.ok(!filer.get_by_id(join(real_pkg, 'index.js'))?.contents, 'unread at first');

			await writeFile(importer_path, `import 'pkg';\nimport '${added}';\n`);
			on_change!({ type: 'update', path: importer_path, is_directory: false });
			await new Promise((r) => setTimeout(r, 20));

			const pkg_node = filer.get_by_id(join(real_pkg, 'index.js'));
			assert.ok(pkg_node?.contents, `read after adding ${added}`);
			assert.ok(pkg_node.dependencies.has(join(real_pkg, 'other.js')));

			await filer.close();
		} finally {
			await rm(dir, { recursive: true, force: true });
		}
	}
});

test('cascading invalidation through dependency chain', async () => {
	// Create dependency chain: a.ts <- b.ts <- c.ts
	const fileA = create_test_disknode('/test/a.ts', 'export const a = 1');
	const fileB = create_test_disknode('/test/b.ts', "import {a} from './a'");
	const fileC = create_test_disknode('/test/c.ts', "import {b} from './b'");

	// Set up dependency chain
	fileB.dependencies.set(fileA.id, fileA);
	fileA.dependents.set(fileB.id, fileB);

	fileC.dependencies.set(fileB.id, fileB);
	fileB.dependents.set(fileC.id, fileC);

	const get_by_id = (id: string) => {
		if (id === fileA.id) return fileA;
		if (id === fileB.id) return fileB;
		if (id === fileC.id) return fileC;
		return undefined;
	};

	// When A changes, both B and C should be invalidated
	const dependents = filter_dependents(fileA, get_by_id);

	assert.equal(dependents.size, 2);
	assert.ok(dependents.has(fileB.id));
	assert.ok(dependents.has(fileC.id));
});

test('deleted file with dependents stays in memory', async () => {
	let on_change_callback: ((change: any) => void) | null = null as any;

	const mock_watch_dir = vi.fn((options) => {
		on_change_callback = options.on_change;
		const mock_watcher: WatchNodeFs = {
			init: vi.fn(async () => {
				options.on_change({ type: 'add', path: '/test/a.ts', is_directory: false });
				options.on_change({ type: 'add', path: '/test/b.ts', is_directory: false });
			}),
			close: vi.fn(async () => {})
		};
		return mock_watcher;
	});

	const filer = new Filer({ watch_dir: mock_watch_dir });
	await filer.init();

	const file_a = filer.get_by_id('/test/a.ts');
	const file_b = filer.get_by_id('/test/b.ts');

	assert.ok(file_a);
	assert.ok(file_b);

	// Manually set up dependency: B depends on A
	file_b.dependencies.set(file_a.id, file_a);
	file_a.dependents.set(file_b.id, file_b);

	const size_before = filer.files.size;

	// Delete A while B still depends on it
	assert.ok(on_change_callback);
	on_change_callback({ type: 'delete', path: '/test/a.ts', is_directory: false });

	// Wait for queue processing
	await new Promise((resolve) => setTimeout(resolve, 10));

	// A should still be in memory since B depends on it
	const file_a_after = filer.get_by_id('/test/a.ts');
	assert.ok(file_a_after, 'File A should still exist since B depends on it');
	assert.equal(file_a_after.contents, null, 'Contents should be cleared');

	// File should still be in the map
	assert.ok(filer.files.has('/test/a.ts'));

	// File count should not change since A is kept in memory due to dependent
	assert.equal(filer.files.size, size_before, 'File count should remain the same');

	// B should still have A as a dependency (dependency tracking preserved)
	assert.ok(file_a_after.dependents.has('/test/b.ts'));
});

test('a deleted file is no longer a dependent of what it imported', async () => {
	const dir = await mkdtemp(join(tmpdir(), 'gro_filer_remove_'));
	try {
		const dep_path = join(dir, 'b.ts');
		const importer_path = join(dir, 'a.gen.ts');
		await writeFile(dep_path, 'export const b = 1;\n');
		await writeFile(importer_path, "import {b} from './b.ts';\nexport const gen = () => b;\n");

		let on_change_callback: ((change: any) => void) | null = null as any;
		const mock_watch_dir = vi.fn((options) => {
			on_change_callback = options.on_change;
			const mock_watcher: WatchNodeFs = {
				init: vi.fn(async () => {
					options.on_change({ type: 'add', path: dep_path, is_directory: false });
					options.on_change({ type: 'add', path: importer_path, is_directory: false });
				}),
				close: vi.fn(async () => {})
			};
			return mock_watcher;
		});

		const filer = new Filer({ watch_dir: mock_watch_dir, watch_dir_options: { dir } });
		await filer.init();
		const dep = filer.get_by_id(dep_path);
		assert.ok(dep);
		assert.ok(filter_dependents(dep, filer.get_by_id).has(importer_path));

		await rm(importer_path);
		assert.ok(on_change_callback);
		on_change_callback({ type: 'delete', path: importer_path, is_directory: false });
		await new Promise((r) => setTimeout(r, 20));

		assert.ok(!dep.dependents.has(importer_path), 'the deleted importer is no longer a dependent');
		assert.equal(filter_dependents(dep, filer.get_by_id).size, 0);
		assert.ok(!filer.files.has(importer_path), 'the deleted file with no dependents is dropped');

		await filer.close();
	} finally {
		await rm(dir, { recursive: true, force: true });
	}
});

test('a burst of synchronous deletes notifies every one', async () => {
	const dir = await mkdtemp(join(tmpdir(), 'gro_filer_delete_burst_'));
	try {
		const ids = ['a.ts', 'b.ts', 'c.ts'].map((f) => join(dir, f));
		for (const id of ids) await writeFile(id, `export const x = '${id}';\n`);
		let on_change_callback: ((change: any) => void) | null = null as any;
		const mock_watch_dir = vi.fn((options) => {
			on_change_callback = options.on_change;
			const mock_watcher: WatchNodeFs = {
				init: vi.fn(async () => {
					for (const id of ids) options.on_change({ type: 'add', path: id, is_directory: false });
				}),
				close: vi.fn(async () => {})
			};
			return mock_watcher;
		});
		const filer = new Filer({ watch_dir: mock_watch_dir, watch_dir_options: { dir } });
		const deleted: Array<string> = [];
		await filer.watch((change) => {
			if (change.type === 'delete') deleted.push(change.path);
		});

		for (const id of ids) await rm(id);
		assert.ok(on_change_callback);
		// as chokidar emits pending unlinks, in one synchronous loop
		for (const id of ids) on_change_callback({ type: 'delete', path: id, is_directory: false });
		await new Promise((r) => setTimeout(r, 20));

		assert.deepEqual(deleted, ids);
		await filer.close();
	} finally {
		await rm(dir, { recursive: true, force: true });
	}
});
