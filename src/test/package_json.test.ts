import {test, expect} from 'vitest';
import {PackageJson, PackageJsonExports} from '@fuzdev/fuz_util/package_json.ts';

import {
	package_json_load,
	package_json_parse_repo_url,
	package_json_serialize,
	package_json_to_exports,
} from '$lib/package_json.ts';

test('package_json_load', async () => {
	const package_json = await package_json_load();
	expect(package_json).toBeTruthy();
	const parsed = PackageJson.parse(package_json);
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

test('PackageJson.parse', () => {
	PackageJson.parse({name: 'abc', version: '123'});
});

test('PackageJson.parse fails with bad data', () => {
	let err;
	try {
		PackageJson.parse({version: '123'});
	} catch (_err) {
		err = _err;
	}
	expect(err).toBeTruthy();
});

test('package_json_serialize', () => {
	package_json_serialize({name: 'abc', version: '123'});
});

test('package_json_serialize fails with bad data', () => {
	let err;
	try {
		package_json_serialize({version: '123'} as any);
	} catch (_err) {
		err = _err;
	}
	expect(err).toBeTruthy();
});

test('package_json_to_exports', () => {
	expect(package_json_to_exports(['a/b.ts'])).toEqual({
		'./package.json': './package.json',
		'./*.js': {
			default: './dist/*.js',
			types: './dist/*.d.ts',
		},
		'./*.ts': {
			default: './dist/*.js',
			types: './dist/*.d.ts',
		},
	});
	expect(package_json_to_exports(['*.svelte', '*.ts', '*.json', 'index.ts'])).toEqual({
		'.': {
			default: './dist/index.js',
			types: './dist/index.d.ts',
		},
		'./package.json': './package.json',
		'./*.json': {
			default: './dist/*.json',
			types: './dist/*.json.d.ts',
		},
		'./*.svelte': {
			svelte: './dist/*.svelte',
			default: './dist/*.svelte',
			types: './dist/*.svelte.d.ts',
		},
		'./*.js': {
			default: './dist/*.js',
			types: './dist/*.d.ts',
		},
		'./*.ts': {
			default: './dist/*.js',
			types: './dist/*.d.ts',
		},
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
		'./lib': './lib/index.js',
	};
	const parsed = PackageJsonExports.safeParse(exports);
	expect(parsed.success).toBe(true);
	expect(exports).toEqual(parsed.data);
});

test('`PackageJsonExports` parses null exports', () => {
	const exports = {
		'.': './index.js',
		'./internal/*': null,
	};
	const parsed = PackageJsonExports.safeParse(exports);
	expect(parsed.success).toBe(true);
	expect(exports).toEqual(parsed.data);
});

test('`PackageJsonExports` parses basic conditional exports', () => {
	const exports = {
		'.': {
			import: './index.mjs',
			require: './index.cjs',
			default: './index.js',
		},
	};
	const parsed = PackageJsonExports.safeParse(exports);
	expect(parsed.success).toBe(true);
	expect(exports).toEqual(parsed.data);
});

test('`PackageJsonExports` parses nested conditional exports', () => {
	const exports = {
		'./feature': {
			node: {
				import: './feature-node.mjs',
				require: './feature-node.cjs',
			},
			default: './feature.mjs',
		},
	};
	const parsed = PackageJsonExports.safeParse(exports);
	expect(parsed.success).toBe(true);
	expect(exports).toEqual(parsed.data);
});

test('`PackageJsonExports` parses deeply nested conditional exports', () => {
	const exports = {
		'./advanced': {
			node: {
				development: {
					import: './dev-node.mjs',
					require: './dev-node.cjs',
				},
				production: {
					import: './prod-node.mjs',
					require: './prod-node.cjs',
				},
			},
			default: './feature.mjs',
		},
	};
	const parsed = PackageJsonExports.safeParse(exports);
	expect(parsed.success).toBe(true);
	expect(exports).toEqual(parsed.data);
});

test('`PackageJsonExports` parses mixed exports types', () => {
	const exports = {
		'.': './index.js',
		'./lib': {
			node: './lib/node.js',
			default: './lib/index.js',
		},
		'./feature/*': null,
		'./advanced': {
			node: {
				import: './advanced-node.mjs',
				require: './advanced-node.cjs',
			},
		},
	};
	const parsed = PackageJsonExports.safeParse(exports);
	expect(parsed.success).toBe(true);
	expect(exports).toEqual(parsed.data);
});

test('rejects invalid exports', () => {
	const invalid_exports = [
		{
			'.': true, // boolean is not a valid export value
		},
		{
			'.': ['/path'], // array is not a valid export value
		},
		{
			'.': {
				default: true, // boolean is not a valid export value in conditions
			},
		},
		{
			'.': {
				node: {
					import: ['/path'], // array is not a valid nested export value
				},
			},
		},
	];

	for (const invalid_export of invalid_exports) {
		const parsed = PackageJsonExports.safeParse(invalid_export);
		expect(parsed.success).toBe(false);
	}
});
