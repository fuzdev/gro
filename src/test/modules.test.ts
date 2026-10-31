import { test, expect } from 'vitest';
import { join, resolve } from 'node:path';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { spawnSync } from 'node:child_process';
import { pathToFileURL } from 'node:url';

import { load_module } from '#lib/modules.ts';

// TODO if we import directly, svelte-package generates types in `src/test/fixtures`

const mod_test1 = await import('./fixtures/' + 'test1.foo.js');

test('load_module basic behavior', async () => {
	const id = resolve('src/test/fixtures/test1.foo.js');
	let validated_mod;
	const result = await load_module(id, (mod): mod is any => {
		validated_mod = mod;
		return true;
	});
	expect(result.ok).toBe(true);
	if (result.ok) {
		expect(result.id).toBe(id);
		expect(result.mod).toBe(validated_mod);
		expect(result.mod).toBe(mod_test1);
	}
});

test('load_module without validation', async () => {
	const id = resolve('src/test/fixtures/test1.foo.js');
	const result = await load_module(id);
	expect(result.ok).toBe(true);
	if (result.ok) {
		expect(result.id).toBe(id);
		expect(result.mod).toBe(mod_test1);
	}
});

test('load_module fails validation', async () => {
	const id = resolve('src/test/fixtures/test1.foo.js');
	let validated_mod;
	const test_validation = (mod: Record<string, any>) => {
		validated_mod = mod;
		return false;
	};
	const result = await load_module(id, test_validation as any);
	expect(result.ok).toBe(false);
	if (!result.ok && result.type === 'failed_validation') {
		expect(result.validation).toBe(test_validation.name);
		expect(result.id).toBe(id);
		expect(result.mod).toBe(validated_mod);
		expect(result.mod).toBe(mod_test1);
	} else {
		throw Error('Should be invalid');
	}
});

test('load_module fails to import', async () => {
	const id = resolve('foo/test/failure');
	const result = await load_module(id);
	expect(result.ok).toBe(false);
	if (!result.ok && result.type === 'failed_import') {
		expect(result.id).toBe(id);
		expect(result.error instanceof Error).toBe(true);
	} else {
		throw Error('Should fail to import');
	}
});

test('load_module imports a path with URL syntax as written, without the loader', async () => {
	const dir = await mkdtemp(join(tmpdir(), 'gro_load_module_#?%25_'));
	try {
		const id = join(dir, 'a #b?c%20.js');
		await writeFile(id, 'export const a = 1;\n');
		// spawned without the loader, which reads a path specifier literally, and outside vitest,
		// whose module runner never decodes a file URL
		const result = spawnSync(
			process.execPath,
			[
				'--experimental-strip-types',
				'--disable-warning=ExperimentalWarning',
				'--input-type=module',
				'-e',
				`const {load_module} = await import(${JSON.stringify(pathToFileURL(resolve('src/lib/modules.ts')).href)});
				const loaded = await load_module(${JSON.stringify(id)});
				console.log(loaded.ok ? loaded.mod.a : loaded.error.message);`
			],
			{ encoding: 'utf8' }
		);
		expect(result.stdout.trim()).toBe('1');
	} finally {
		await rm(dir, { recursive: true, force: true });
	}
});
