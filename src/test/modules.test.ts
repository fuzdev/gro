import { test, expect, assert } from 'vitest';
import { join, resolve } from 'node:path';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { spawnSync } from 'node:child_process';
import { pathToFileURL } from 'node:url';

import { load_module, load_modules, type ImportModule } from '#lib/modules.ts';
import { import_module_node } from '#lib/internal/node_import_module.ts';
import { paths } from './test_helpers.ts';

// TODO if we import directly, svelte-package generates types in `src/test/fixtures`

const mod_test1 = await import('./fixtures/' + 'test1.foo.js');

test('load_module basic behavior', async () => {
	const id = resolve('src/test/fixtures/test1.foo.js');
	let validated_mod;
	const result = await load_module(id, import_module_node, (mod): mod is any => {
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
	const result = await load_module(id, import_module_node);
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
	const result = await load_module(id, import_module_node, test_validation as any);
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
	const result = await load_module(id, import_module_node);
	expect(result.ok).toBe(false);
	if (!result.ok && result.type === 'failed_import') {
		expect(result.id).toBe(id);
		expect(result.error instanceof Error).toBe(true);
	} else {
		throw Error('Should fail to import');
	}
});

test('import_module_node imports a path with URL syntax as written, without the loader', async () => {
	const dir = await mkdtemp(join(tmpdir(), 'gro_load_module_#?%25_'));
	try {
		const id = join(dir, 'a #b?c%20.js');
		await writeFile(id, 'export const a = 1;\n');
		// spawned without the loader, which reads a path specifier literally, and outside vitest,
		// whose module runner never decodes a file URL
		const result = spawnSync(
			process.execPath,
			[
				'--input-type=module',
				'-e',
				`const {load_module} = await import(${JSON.stringify(pathToFileURL(resolve('src/lib/modules.ts')).href)});
				const {import_module_node} = await import(${JSON.stringify(pathToFileURL(resolve('src/lib/internal/node_import_module.ts')).href)});
				const loaded = await load_module(${JSON.stringify(id)}, import_module_node);
				console.log(loaded.ok ? loaded.mod.a : loaded.error.message);`
			],
			{ encoding: 'utf8' }
		);
		expect(result.stdout.trim()).toBe('1');
	} finally {
		await rm(dir, { recursive: true, force: true });
	}
});

test('load_modules reports a module whose import throws with its stack, then its causes', async () => {
	const dir = await mkdtemp(join(tmpdir(), 'gro_modules_cause_'));
	try {
		const id = join(dir, 'throws.mjs');
		await writeFile(
			id,
			"throw new Error('io_error: /p/x', { cause: new Error('too many symlinks at /p/x') });\n"
		);
		const result = await load_modules(
			[{ id, input_path: id, resolved_input_path: null as any }],
			(mod): mod is Record<string, unknown> => !!mod,
			(_file, mod) => ({ id, mod }),
			paths.root,
			import_module_node
		);
		expect(result.ok).toBe(false);
		if (result.ok) return;
		expect(result.reasons).toHaveLength(1);
		expect(result.reasons[0]).toMatch(
			/Error: io_error: \/p\/x\n\s+at [^]*\ncause: too many symlinks at \/p\/x/
		);
	} finally {
		await rm(dir, { recursive: true, force: true });
	}
});

test('load_module loads through the `import_module` it is given', async () => {
	const namespace = { a: 1 };
	const ids: Array<string> = [];
	const import_module: ImportModule = (id) => {
		ids.push(id);
		return Promise.resolve(namespace);
	};
	const result = await load_module('/p/no/such/file.ts', import_module);
	assert.ok(result.ok);
	assert.strictEqual(result.mod, namespace);
	assert.deepEqual(ids, ['/p/no/such/file.ts']);
});

test('load_module reports what `import_module` rejects with as a failed import', async () => {
	const error = new Error('refused');
	const result = await load_module('/p/x.ts', () => Promise.reject(error));
	assert.ok(!result.ok);
	assert.strictEqual(result.type, 'failed_import');
	if (result.type === 'failed_import') assert.strictEqual(result.error, error);
});

test('load_modules loads each file through `import_module`', async () => {
	const ids: Array<string> = [];
	const result = await load_modules(
		['/p/a.gen.ts', '/p/b.gen.ts'].map((id) => ({
			id,
			input_path: id,
			resolved_input_path: null as any
		})),
		(mod): mod is { gen: string } => typeof mod.gen === 'string',
		(file, mod) => ({ id: file.id, mod }),
		'/p',
		(id) => {
			ids.push(id);
			return Promise.resolve({ gen: id });
		}
	);
	assert.ok(result.ok);
	assert.deepEqual(
		result.modules.map((m) => m.mod.gen),
		['/p/a.gen.ts', '/p/b.gen.ts']
	);
	assert.deepEqual(ids, ['/p/a.gen.ts', '/p/b.gen.ts']);
});
