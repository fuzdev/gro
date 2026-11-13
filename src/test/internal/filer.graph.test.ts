import { test, assert, vi } from 'vitest';
import { paths } from '#lib/paths.ts';
import { dirname, join, resolve } from 'node:path';
import { mkdir, mkdtemp, realpath, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';

import type { WatchNodeFs, WatcherChange } from '#lib/internal/watch_dir.ts';
import { Filer, filter_dependents } from '#lib/internal/filer.ts';

import { create_test_disknode } from './filer_test_helpers.ts';

const fixtures_dir = resolve(import.meta.dirname, '../fixtures');

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

	const filer = new Filer({ dir: paths.source, watch_dir: mock_watch_dir });
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

	const filer = new Filer({ dir: paths.source, watch_dir: mock_watch_dir });
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

		const filer = new Filer({ dir: paths.source, watch_dir: mock_watch_dir });
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
			dir: src,
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

test('a package file in `node_modules` is never read, whether a subpath mapping or a path reaches it', async () => {
	const dir = await mkdtemp(join(tmpdir(), 'gro_filer_node_modules_'));
	try {
		const src = join(dir, 'src');
		const dep = join(dir, 'node_modules/dep');
		await mkdir(src);
		await mkdir(dep, { recursive: true });
		await writeFile(
			join(dir, 'package.json'),
			JSON.stringify({ name: 'proj', type: 'module', imports: { '#dep': 'dep' } })
		);
		await writeFile(
			join(dep, 'package.json'),
			JSON.stringify({ name: 'dep', type: 'module', exports: './index.js' })
		);
		// an optional peer that isn't installed, which a read would fail to resolve
		await writeFile(join(dep, 'index.js'), "import './other.js';\nimport 'missing_peer';\n");
		await writeFile(join(dep, 'side.js'), "import 'missing_peer';\n");
		await writeFile(join(dep, 'other.js'), 'export {};\n');
		const importer_path = join(src, 'a.ts');
		await writeFile(importer_path, "import '#dep';\nimport '../node_modules/dep/side.js';\n");

		const log = { error: vi.fn(), warn: vi.fn(), info: vi.fn(), debug: vi.fn() };
		const filer = new Filer({
			watch_dir: (options) => ({
				init: async () => {
					options.on_change({ type: 'add', path: importer_path, is_directory: false });
				},
				close: async () => {}
			}),
			dir: src,
			log: log as any
		});
		await filer.init();
		await new Promise((r) => setTimeout(r, 20));

		const real_dep = join(await realpath(dir), 'node_modules/dep');
		for (const dep_path of [join(real_dep, 'index.js'), join(real_dep, 'side.js')]) {
			const dep_node = filer.get_by_id(dep_path);
			assert.ok(dep_node, `the edge to ${dep_path} is recorded`);
			assert.ok(dep_node.dependents.has(importer_path));
			assert.equal(dep_node.contents, null, `${dep_path} is never read`);
			assert.equal(dep_node.dependencies.size, 0, `${dep_path} is never parsed`);
		}
		assert.equal(filer.get_by_id(join(real_dep, 'other.js')), undefined);
		assert.deepEqual(log.error.mock.calls, []);

		await filer.close();
	} finally {
		await rm(dir, { recursive: true, force: true });
	}
});

/**
 * Inits a Filer on `dir` whose watcher adds only `importer_path`, and waits for its imports.
 */
const init_filer_with_importer = async (dir: string, importer_path: string): Promise<Filer> => {
	const filer = new Filer({
		watch_dir: (options) => ({
			init: async () => {
				options.on_change({ type: 'add', path: importer_path, is_directory: false });
			},
			close: async () => {}
		}),
		dir
	});
	await filer.init();
	await new Promise((r) => setTimeout(r, 20));
	return filer;
};

/**
 * Writes a project at `proj` with `#lib/*` mapped to `./lib/*`, `#dep` to its own installed
 * `dep`, and `#hoisted` to `hoisted`, which only a `node_modules` holding `proj` provides.
 */
const write_project_with_packages = async (proj: string): Promise<void> => {
	await mkdir(join(proj, 'src'), { recursive: true });
	await mkdir(join(proj, 'lib'));
	await writeFile(
		join(proj, 'package.json'),
		JSON.stringify({
			name: 'app',
			type: 'module',
			imports: { '#lib/*': './lib/*', '#dep': 'dep', '#hoisted': 'hoisted' }
		})
	);
	await writeFile(join(proj, 'lib/c.ts'), "import './c_dep.ts';\n");
	await writeFile(join(proj, 'lib/d.ts'), "import './d_dep.ts';\n");
	for (const pkg of [join(proj, 'node_modules/dep'), join(proj, '../hoisted')]) {
		await mkdir(pkg, { recursive: true });
		await writeFile(
			join(pkg, 'package.json'),
			JSON.stringify({ name: 'pkg', type: 'module', exports: './index.js' })
		);
		await writeFile(join(pkg, 'index.js'), "import './other.js';\n");
	}
	await writeFile(
		join(proj, 'src/a.ts'),
		"import '../lib/c.ts';\nimport '#lib/d.ts';\nimport '#dep';\nimport '#hoisted';\n"
	);
};

test('a project in a `node_modules` directory reads its own files, and judges packages below it', async () => {
	const dir = await mkdtemp(join(tmpdir(), 'gro_filer_node_modules_root_'));
	try {
		const proj = join(dir, 'node_modules/app');
		await write_project_with_packages(proj);
		const filer = await init_filer_with_importer(join(proj, 'src'), join(proj, 'src/a.ts'));

		const real = join(await realpath(dir), 'node_modules');
		for (const [id, dep] of [
			['app/lib/c.ts', 'app/lib/c_dep.ts'], // a path climbing out of the watched directory
			['app/lib/d.ts', 'app/lib/d_dep.ts'], // a subpath import
			// a package beside the project, which the path from the watched directory reaches
			// without passing through a `node_modules`
			['hoisted/index.js', 'hoisted/other.js']
		] as const) {
			const node = filer.get_by_id(join(real, id));
			assert.ok(node?.contents, `${id} is read`);
			assert.ok(node.dependencies.has(join(real, dep)), `${id} is parsed`);
		}
		const own_dep = filer.get_by_id(join(real, 'app/node_modules/dep/index.js'));
		assert.ok(own_dep, "the edge to the project's own package is recorded");
		assert.equal(own_dep.contents, null, "the project's own package is never read");

		await filer.close();
	} finally {
		await rm(dir, { recursive: true, force: true });
	}
});

test('a project reached through a symlink judges a path from the link and a real path from its target', async () => {
	// a link to a project in a `node_modules`, and a project linked into one, as a workspace is
	for (const [target, link] of [
		['real/node_modules/app', 'link'],
		['real/app', 'node_modules/app']
	] as const) {
		const dir = await mkdtemp(join(tmpdir(), 'gro_filer_symlinked_root_'));
		try {
			const proj = join(dir, target);
			await write_project_with_packages(proj);
			await mkdir(dirname(join(dir, link)), { recursive: true });
			await symlink(proj, join(dir, link));
			const linked = join(dir, link);
			const filer = await init_filer_with_importer(join(linked, 'src'), join(linked, 'src/a.ts'));

			// a path import keeps the link's path
			const c = filer.get_by_id(join(linked, 'lib/c.ts'));
			assert.ok(c?.contents, `a path import of a project file is read through ${link}`);
			// a subpath import resolves to the real path
			const real = join(await realpath(dir), target);
			const d = filer.get_by_id(join(real, 'lib/d.ts'));
			assert.ok(d?.contents, `a subpath import of a project file is read through ${link}`);
			assert.ok(d.dependencies.has(join(real, 'lib/d_dep.ts')));
			const own_dep = filer.get_by_id(join(real, 'node_modules/dep/index.js'));
			assert.ok(own_dep, "the edge to the project's own package is recorded");
			assert.equal(
				own_dep.contents,
				null,
				`the project's own package is never read through ${link}`
			);

			await filer.close();
		} finally {
			await rm(dir, { recursive: true, force: true });
		}
	}
});

test('a directory whose name only contains `node_modules` holds no package', async () => {
	const dir = await mkdtemp(join(tmpdir(), 'gro_filer_node_modules_x_'));
	try {
		await mkdir(join(dir, 'src'));
		const ids = ['node_modules_x/y.js', 'x_node_modules/y.js'];
		for (const id of ids) {
			await mkdir(dirname(join(dir, id)), { recursive: true });
			await writeFile(join(dir, id), "import './z.js';\n");
		}
		const importer_path = join(dir, 'src/a.ts');
		await writeFile(importer_path, ids.map((id) => `import '../${id}';\n`).join(''));
		const filer = await init_filer_with_importer(join(dir, 'src'), importer_path);

		for (const id of ids) {
			const node = filer.get_by_id(join(dir, id));
			assert.ok(node?.contents, `${id} is read`);
			assert.ok(node.dependencies.has(join(dir, dirname(id), 'z.js')), `${id} is parsed`);
		}

		await filer.close();
	} finally {
		await rm(dir, { recursive: true, force: true });
	}
});

test('a `#lib/*` import of a project file outside the watched directory is read', async () => {
	const dir = await mkdtemp(join(tmpdir(), 'gro_filer_subpath_lib_'));
	try {
		const routes = join(dir, 'src/routes');
		const lib = join(dir, 'src/lib');
		await mkdir(routes, { recursive: true });
		await mkdir(lib);
		await writeFile(
			join(dir, 'package.json'),
			JSON.stringify({ name: 'proj', type: 'module', imports: { '#lib/*': './src/lib/*' } })
		);
		await writeFile(join(lib, 'b.ts'), "import './c.ts';\n");
		await writeFile(join(lib, 'c.ts'), 'export {};\n');
		const importer_path = join(routes, 'a.ts');
		await writeFile(importer_path, "import '#lib/b.ts';\n");
		const filer = new Filer({
			watch_dir: (options) => ({
				init: async () => {
					options.on_change({ type: 'add', path: importer_path, is_directory: false });
				},
				close: async () => {}
			}),
			dir: routes
		});
		await filer.init();
		await new Promise((r) => setTimeout(r, 20));

		const real_lib = join(await realpath(dir), 'src/lib');
		const b = filer.get_by_id(join(real_lib, 'b.ts'));
		assert.ok(b?.external);
		assert.ok(b.contents, 'a subpath import of a project file is read');
		assert.ok(b.dependencies.has(join(real_lib, 'c.ts')));

		await filer.close();
	} finally {
		await rm(dir, { recursive: true, force: true });
	}
});

/**
 * Writes a project whose `pkg` is linked from `linked_pkg/`, so its file realpaths outside
 * `node_modules`, where a path or `#p` import of it reads it and a bare `pkg` import doesn't.
 */
const write_linked_pkg_project = async (dir: string): Promise<void> => {
	const linked = join(dir, 'linked_pkg');
	await mkdir(join(dir, 'src'));
	await mkdir(linked);
	await mkdir(join(dir, 'node_modules'));
	await symlink(linked, join(dir, 'node_modules/pkg'));
	await writeFile(
		join(dir, 'package.json'),
		JSON.stringify({ name: 'proj', type: 'module', imports: { '#p': 'pkg' } })
	);
	await writeFile(
		join(linked, 'package.json'),
		JSON.stringify({ name: 'pkg', type: 'module', exports: './index.js' })
	);
	await writeFile(join(linked, 'index.js'), "import './other.js';\n");
	await writeFile(join(linked, 'other.js'), 'export {};\n');
};

test('a file reached by both a bare specifier and a path or subpath import is read in either order', async () => {
	for (const order of [
		['pkg', '#p'],
		['#p', 'pkg'],
		['pkg', '../linked_pkg/index.js'],
		['../linked_pkg/index.js', 'pkg']
	]) {
		const dir = await mkdtemp(join(tmpdir(), 'gro_filer_read_order_'));
		try {
			await write_linked_pkg_project(dir);
			const importer_path = join(dir, 'src/a.ts');
			await writeFile(importer_path, order.map((sp) => `import '${sp}';`).join('\n'));
			const filer = new Filer({
				watch_dir: (options) => ({
					init: async () => {
						options.on_change({ type: 'add', path: importer_path, is_directory: false });
					},
					close: async () => {}
				}),
				dir: join(dir, 'src')
			});
			await filer.init();
			await new Promise((r) => setTimeout(r, 20));

			const real_pkg = join(await realpath(dir), 'linked_pkg');
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
	for (const added of ['#p', '../linked_pkg/index.js']) {
		const dir = await mkdtemp(join(tmpdir(), 'gro_filer_read_on_update_'));
		try {
			await write_linked_pkg_project(dir);
			const importer_path = join(dir, 'src/a.ts');
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
				dir: join(dir, 'src')
			});
			await filer.init();
			await new Promise((r) => setTimeout(r, 20));

			const real_pkg = join(await realpath(dir), 'linked_pkg');
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

	const filer = new Filer({ dir: paths.source, watch_dir: mock_watch_dir });
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

		const filer = new Filer({ watch_dir: mock_watch_dir, dir });
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
		const filer = new Filer({ watch_dir: mock_watch_dir, dir });
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
