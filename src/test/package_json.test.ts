import { describe, test, expect, assert } from 'vitest';
import { stripVTControlCharacters } from 'node:util';
import * as s from '@fuzdev/fuz_schema/schema.ts';
import { PackageJson, PackageJsonExports } from '@fuzdev/fuz_util/package_json.ts';

import {
	package_json_has_dependency,
	package_json_load,
	package_json_parse_repo_url,
	package_json_serialize,
	package_json_to_exports
} from '#lib/package_json.ts';

test('package_json_load', async () => {
	const package_json = await package_json_load();
	expect(package_json).toBeTruthy();
	const parsed = s.parse_or_throw(PackageJson, package_json);
	expect(parsed).toBeTruthy();
	package_json_serialize(package_json);
});

test('package_json_load with cache', async () => {
	const cache = {};
	const package_json1 = await package_json_load(undefined, cache);
	expect(package_json1).toBeTruthy();
	expect(Object.keys(cache).length).toBe(1);
	const package_json2 = await package_json_load(undefined, cache);
	expect(Object.keys(cache).length).toBe(1);
	expect(package_json1).toBe(package_json2);
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
	const parsed = package_json_parse_repo_url(await package_json_load());
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
