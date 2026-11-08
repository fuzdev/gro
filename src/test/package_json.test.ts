import { describe, test, expect, assert } from 'vitest';
import { stripVTControlCharacters } from 'node:util';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import * as s from '@fuzdev/fuz_schema/schema.ts';
import { PackageJson, PackageJsonExports } from '@fuzdev/fuz_util/package_json.ts';

import {
	PACKAGE_JSON_EMPTY,
	package_json_has_dependency,
	package_json_load,
	package_json_load_for_gro,
	package_json_parse_repo_url,
	package_json_serialize,
	package_json_sync,
	package_json_to_exports,
	package_json_update
} from '#lib/package_json.ts';
import {
	create_fs_sandbox,
	create_project_fs_paths,
	remove_fs_sandbox
} from './fs_test_helpers.ts';
import { create_mock_fs_dir, create_mock_project_fs_paths, mock_tree } from './fs_mocks.ts';
import { create_mock_logger } from './test_helpers.ts';

// this repo's own `package.json`
const repo_fs_paths = await create_project_fs_paths(process.cwd());

test('package_json_load', async () => {
	const package_json = await package_json_load(repo_fs_paths);
	expect(package_json).toBeTruthy();
	const parsed = s.parse_or_throw(PackageJson, package_json);
	expect(parsed).toBeTruthy();
	package_json_serialize(package_json);
});

test('package_json_load with cache', async () => {
	const cache = {};
	const package_json1 = await package_json_load(repo_fs_paths, undefined, cache);
	expect(package_json1).toBeTruthy();
	expect(Object.keys(cache).length).toBe(1);
	const package_json2 = await package_json_load(repo_fs_paths, undefined, cache);
	expect(Object.keys(cache).length).toBe(1);
	expect(package_json1).toBe(package_json2);
});

test('package_json_load reads `dir` relative to the project', async () => {
	const project = create_mock_fs_dir('/project');
	mock_tree(project, { 'sub/package.json': '{"name": "a", "version": "1.0.0"}' });
	const package_json = await package_json_load(create_mock_project_fs_paths(project), 'sub');
	assert.deepEqual(package_json, { name: 'a', version: '1.0.0' });
});

test('package_json_load returns the empty package.json when it is missing', async () => {
	const project = create_mock_fs_dir('/project');
	mock_tree(project, {});
	const package_json = await package_json_load(create_mock_project_fs_paths(project));
	assert.strictEqual(package_json, PACKAGE_JSON_EMPTY);
});

test('package_json_load_for_gro reads the running Gro through the module system', async () => {
	const gro_package_json = package_json_load_for_gro();
	assert.strictEqual(gro_package_json.name, '@fuzdev/gro');
	assert.strictEqual(
		gro_package_json.version,
		(await package_json_load(repo_fs_paths)).version // this repo is the running Gro
	);
});

test('package_json_update writes the package.json in `dir`', async () => {
	const project = create_mock_fs_dir('/project');
	mock_tree(project, { 'sub/package.json': '{"name": "a", "version": "1.0.0"}' });
	project.write.mockResolvedValue(undefined);
	const updated = await package_json_update(
		create_mock_project_fs_paths(project),
		(package_json) => ({ ...package_json, version: '2.0.0' }),
		'sub'
	);
	assert.ok(updated.changed);
	assert.strictEqual(project.write.mock.calls.length, 1);
	const [path, contents] = project.write.mock.calls[0]!;
	assert.strictEqual(path, 'sub/package.json');
	assert.strictEqual(contents, '{\n  "name": "a",\n  "version": "2.0.0"\n}\n');
});

test('package_json_update writes nothing when the contents are unchanged', async () => {
	const project = create_mock_fs_dir('/project');
	mock_tree(project, { 'package.json': '{\n  "name": "a",\n  "version": "1.0.0"\n}\n' });
	const updated = await package_json_update(create_mock_project_fs_paths(project), (p) => p);
	assert.ok(!updated.changed);
	assert.strictEqual(project.write.mock.calls.length, 0);
});

describe('package_json_sync', () => {
	const write_library = (
		root: string,
		devDependencies: Record<string, string> = { '@sveltejs/kit': '2', '@sveltejs/package': '2' }
	): void => {
		mkdirSync(join(root, 'src/lib/internal'), { recursive: true });
		writeFileSync(join(root, 'src/lib/a.ts'), '');
		writeFileSync(join(root, 'src/lib/B.svelte'), '');
		writeFileSync(join(root, 'src/lib/internal/c.ts'), '');
		writeFileSync(
			join(root, 'package.json'),
			JSON.stringify({
				name: 'lib',
				version: '1.0.0',
				devDependencies
			})
		);
	};

	test('writes the exports of the lib directory beneath the project', async () => {
		const sandbox = await create_fs_sandbox();
		try {
			write_library(sandbox.root);
			const fs_paths = await create_project_fs_paths(sandbox.root);
			const { changed } = await package_json_sync(fs_paths, (p) => p, create_mock_logger());
			assert.ok(changed);
			const written = JSON.parse(readFileSync(join(sandbox.root, 'package.json'), 'utf8'));
			assert.deepEqual(Object.keys(written.exports), [
				'./package.json',
				'./internal/*',
				'./*.js',
				'./*.ts',
				'./*.svelte'
			]);
			// synced, so a second run changes nothing
			const again = await package_json_sync(fs_paths, (p) => p, create_mock_logger());
			assert.ok(!again.changed);
		} finally {
			remove_fs_sandbox(sandbox);
		}
	});

	test('with `write` false reports a change without writing', async () => {
		const sandbox = await create_fs_sandbox();
		try {
			write_library(sandbox.root);
			const before = readFileSync(join(sandbox.root, 'package.json'), 'utf8');
			const fs_paths = await create_project_fs_paths(sandbox.root);
			const { changed } = await package_json_sync(fs_paths, (p) => p, create_mock_logger(), false);
			assert.ok(changed);
			assert.strictEqual(readFileSync(join(sandbox.root, 'package.json'), 'utf8'), before);
		} finally {
			remove_fs_sandbox(sandbox);
		}
	});

	test('leaves the exports of a project that is not a library', async () => {
		const sandbox = await create_fs_sandbox();
		try {
			write_library(sandbox.root, { '@sveltejs/kit': '2' });
			const fs_paths = await create_project_fs_paths(sandbox.root);
			const { changed } = await package_json_sync(
				fs_paths,
				(p) => ({ ...p, description: 'mapped' }),
				create_mock_logger()
			);
			assert.ok(changed);
			const written = JSON.parse(readFileSync(join(sandbox.root, 'package.json'), 'utf8'));
			assert.strictEqual(written.exports, undefined);
			assert.strictEqual(written.description, 'mapped');
		} finally {
			remove_fs_sandbox(sandbox);
		}
	});
});

test('PackageJson parses', () => {
	s.parse_or_throw(PackageJson, { name: 'abc', version: '123' });
});

test('PackageJson fails with bad data', () => {
	let err;
	try {
		s.parse_or_throw(PackageJson, { version: '123' });
	} catch (_err) {
		err = _err;
	}
	expect(err).toBeTruthy();
});

test('package_json_serialize', () => {
	package_json_serialize({ name: 'abc', version: '123' });
});

test('package_json_serialize fails with bad data', () => {
	let err;
	try {
		package_json_serialize({ version: '123' } as any);
	} catch (_err) {
		err = _err;
	}
	expect(err).toBeTruthy();
});

test('package_json_serialize lists every issue under a header', () => {
	let err;
	try {
		package_json_serialize({ name: 1, dependencies: { x: 2, 'a.b': 3 } } as any);
	} catch (_err) {
		err = _err;
	}
	assert(err instanceof Error);
	assert.strictEqual(
		stripVTControlCharacters(err.message),
		'Failed to parse package.json:\n' +
			'\n\tname: expected string, received number\n' +
			'\n\tdependencies.x: expected string, received number\n' +
			'\n\tdependencies."a.b": expected string, received number\n'
	);
});

describe('package_json_to_exports', () => {
	test('emits wildcard exports for js paths', () => {
		expect(package_json_to_exports(['a/b.ts'])).toEqual({
			'./package.json': './package.json',
			'./*.js': {
				default: './dist/*.js',
				types: './dist/*.d.ts'
			},
			'./*.ts': {
				default: './dist/*.js',
				types: './dist/*.d.ts'
			}
		});
	});

	// the internal/ convention: each internal directory (any depth) emits a
	// null blocker, and internal files don't count toward the wildcard flags
	// (the internal-only .svelte file adds no ./*.svelte wildcard)
	test('blocks internal dirs and excludes their files from the wildcard flags', () => {
		expect(
			package_json_to_exports([
				'a/b.ts',
				'internal/c.ts',
				'internal/D.svelte',
				'domain/internal/e.ts',
				'internal', // a file named internal is public
				'x/internal', // nested or not
				'internal.ts' // dotted names don't match the dir convention
			])
		).toEqual({
			'./package.json': './package.json',
			'./domain/internal/*': null,
			'./internal/*': null,
			'./*.js': {
				default: './dist/*.js',
				types: './dist/*.d.ts'
			},
			'./*.ts': {
				default: './dist/*.js',
				types: './dist/*.d.ts'
			}
		});
	});

	test('collapses nested internal dirs to the outermost blocker, which covers them', () => {
		expect(package_json_to_exports(['a/b.ts', 'internal/x/internal/y.ts'])).toEqual({
			'./package.json': './package.json',
			'./internal/*': null,
			'./*.js': {
				default: './dist/*.js',
				types: './dist/*.d.ts'
			},
			'./*.ts': {
				default: './dist/*.js',
				types: './dist/*.d.ts'
			}
		});
	});

	test('emits blockers but no wildcards for an internal-only library', () => {
		expect(package_json_to_exports(['internal/a.ts', 'domain/internal/B.svelte'])).toEqual({
			'./package.json': './package.json',
			'./domain/internal/*': null,
			'./internal/*': null
		});
	});

	test('emits the index export and per-type wildcards', () => {
		expect(package_json_to_exports(['*.svelte', '*.ts', '*.json', 'index.ts'])).toEqual({
			'.': {
				default: './dist/index.js',
				types: './dist/index.d.ts'
			},
			'./package.json': './package.json',
			'./*.json': {
				default: './dist/*.json',
				types: './dist/*.json.d.ts'
			},
			'./*.svelte': {
				svelte: './dist/*.svelte',
				default: './dist/*.svelte',
				types: './dist/*.svelte.d.ts'
			},
			'./*.js': {
				default: './dist/*.js',
				types: './dist/*.d.ts'
			},
			'./*.ts': {
				default: './dist/*.js',
				types: './dist/*.d.ts'
			}
		});
	});
});

test('package_json_parse_repo_url', async () => {
	const parsed = package_json_parse_repo_url(await package_json_load(repo_fs_paths));
	expect(parsed?.owner).toBe('fuzdev');
	expect(parsed?.repo).toBe('gro');
});

test('`PackageJsonExports` parses simple string exports', () => {
	const exports = {
		'.': './index.js',
		'./lib': './lib/index.js'
	};
	const parsed = s.parse(PackageJsonExports, exports);
	assert.ok(parsed.ok);
	expect(exports).toEqual(parsed.value);
});

test('`PackageJsonExports` parses null exports', () => {
	const exports = {
		'.': './index.js',
		'./internal/*': null
	};
	const parsed = s.parse(PackageJsonExports, exports);
	assert.ok(parsed.ok);
	expect(exports).toEqual(parsed.value);
});

test('`PackageJsonExports` parses basic conditional exports', () => {
	const exports = {
		'.': {
			import: './index.mjs',
			require: './index.cjs',
			default: './index.js'
		}
	};
	const parsed = s.parse(PackageJsonExports, exports);
	assert.ok(parsed.ok);
	expect(exports).toEqual(parsed.value);
});

test('`PackageJsonExports` parses nested conditional exports', () => {
	const exports = {
		'./feature': {
			node: {
				import: './feature-node.mjs',
				require: './feature-node.cjs'
			},
			default: './feature.mjs'
		}
	};
	const parsed = s.parse(PackageJsonExports, exports);
	assert.ok(parsed.ok);
	expect(exports).toEqual(parsed.value);
});

test('`PackageJsonExports` parses deeply nested conditional exports', () => {
	const exports = {
		'./advanced': {
			node: {
				development: {
					import: './dev-node.mjs',
					require: './dev-node.cjs'
				},
				production: {
					import: './prod-node.mjs',
					require: './prod-node.cjs'
				}
			},
			default: './feature.mjs'
		}
	};
	const parsed = s.parse(PackageJsonExports, exports);
	assert.ok(parsed.ok);
	expect(exports).toEqual(parsed.value);
});

test('`PackageJsonExports` parses mixed exports types', () => {
	const exports = {
		'.': './index.js',
		'./lib': {
			node: './lib/node.js',
			default: './lib/index.js'
		},
		'./feature/*': null,
		'./advanced': {
			node: {
				import: './advanced-node.mjs',
				require: './advanced-node.cjs'
			}
		}
	};
	const parsed = s.parse(PackageJsonExports, exports);
	assert.ok(parsed.ok);
	expect(exports).toEqual(parsed.value);
});

test('rejects invalid exports', () => {
	const invalid_exports = [
		{
			'.': true // boolean is not a valid export value
		},
		{
			'.': ['/path'] // array is not a valid export value
		},
		{
			'.': {
				default: true // boolean is not a valid export value in conditions
			}
		},
		{
			'.': {
				node: {
					import: ['/path'] // array is not a valid nested export value
				}
			}
		}
	];

	for (const invalid_export of invalid_exports) {
		const parsed = s.parse(PackageJsonExports, invalid_export);
		expect(parsed.ok).toBe(false);
	}
});

describe('package_json_has_dependency', () => {
	const package_json: PackageJson = {
		name: 'a',
		version: '0',
		dependencies: { dep: '1' },
		devDependencies: { dev_dep: '1' },
		peerDependencies: { peer_dep: '1' }
	};

	test('finds deps and dev deps', () => {
		expect(package_json_has_dependency('dep', package_json)).toBe(true);
		expect(package_json_has_dependency('dev_dep', package_json)).toBe(true);
		expect(package_json_has_dependency('missing', package_json)).toBe(false);
	});

	// Peer deps count by default because most callers ask "is this available at runtime",
	// but detection asks "is this what the project is", and a peer answers neither.
	test('counts peer deps only when `include_peer`', () => {
		expect(package_json_has_dependency('peer_dep', package_json)).toBe(true);
		expect(package_json_has_dependency('peer_dep', package_json, false)).toBe(false);
		expect(package_json_has_dependency('dev_dep', package_json, false)).toBe(true);
	});
});
