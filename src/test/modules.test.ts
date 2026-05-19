import {test, expect} from 'vitest';
import {resolve} from 'node:path';

import {load_module} from '../lib/modules.ts';

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
