import { describe, test, expect, beforeEach, afterEach } from 'vitest';
import { dirname } from 'node:path';

import { sveltekit_tsconfig_is_unsynced } from '#lib/sveltekit_helpers.ts';
import { SVELTEKIT_TSCONFIG_PATH } from '#lib/constants.ts';
import { fs, type FsDir } from '#lib/fs.ts';

describe('sveltekit_tsconfig_is_unsynced', () => {
	let dir: FsDir & AsyncDisposable;

	const write = async (path: string, contents: string): Promise<void> => {
		await dir.mkdir(dirname(path), { recursive: true });
		await dir.write(path, contents);
	};

	beforeEach(async () => {
		dir = await fs.temp_dir('gro_sveltekit_tsconfig_');
	});

	afterEach(async () => {
		await dir[Symbol.asyncDispose]();
	});

	test('a string `$app/tsconfig` extends without the generated tsconfig is unsynced', async () => {
		await write('tsconfig.json', '{"extends": "$app/tsconfig"}');

		expect(await sveltekit_tsconfig_is_unsynced(dir.path)).toBe(true);
	});

	test('reads JSONC and an array `extends`', async () => {
		await write(
			'tsconfig.json',
			`{
				// comments and trailing commas are valid tsconfig
				"extends": ["./tsconfig.base.json", "$app/tsconfig.json",],
			}`
		);

		expect(await sveltekit_tsconfig_is_unsynced(dir.path)).toBe(true);
	});

	test('is synced when the generated tsconfig exists', async () => {
		await write('tsconfig.json', '{"extends": "$app/tsconfig"}');
		await write(SVELTEKIT_TSCONFIG_PATH, '{}');

		expect(await sveltekit_tsconfig_is_unsynced(dir.path)).toBe(false);
	});

	test('ignores a tsconfig that does not extend `$app/tsconfig`', async () => {
		await write('tsconfig.json', '{"extends": "@tsconfig/svelte"}');

		expect(await sveltekit_tsconfig_is_unsynced(dir.path)).toBe(false);
	});

	test('a missing or unparseable tsconfig is not unsynced', async () => {
		expect(await sveltekit_tsconfig_is_unsynced(dir.path)).toBe(false);

		await write('tsconfig.json', 'not json {');
		expect(await sveltekit_tsconfig_is_unsynced(dir.path)).toBe(false);
	});
});
