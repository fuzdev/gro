import { describe, test, expect, beforeEach, afterEach } from 'vitest';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';

import { sveltekit_tsconfig_is_unsynced } from '#lib/sveltekit_helpers.ts';
import { SVELTEKIT_TSCONFIG_PATH } from '#lib/constants.ts';

describe('sveltekit_tsconfig_is_unsynced', () => {
	let dir: string;

	const write = (path: string, contents: string): void => {
		const file = join(dir, path);
		mkdirSync(dirname(file), { recursive: true });
		writeFileSync(file, contents);
	};

	beforeEach(() => {
		dir = mkdtempSync(join(tmpdir(), 'gro_sveltekit_tsconfig_'));
	});

	afterEach(() => {
		rmSync(dir, { recursive: true, force: true });
	});

	test('a string `$app/tsconfig` extends without the generated tsconfig is unsynced', async () => {
		write('tsconfig.json', '{"extends": "$app/tsconfig"}');

		expect(await sveltekit_tsconfig_is_unsynced(dir)).toBe(true);
	});

	test('reads JSONC and an array `extends`', async () => {
		write(
			'tsconfig.json',
			`{
				// comments and trailing commas are valid tsconfig
				"extends": ["./tsconfig.base.json", "$app/tsconfig.json",],
			}`
		);

		expect(await sveltekit_tsconfig_is_unsynced(dir)).toBe(true);
	});

	test('is synced when the generated tsconfig exists', async () => {
		write('tsconfig.json', '{"extends": "$app/tsconfig"}');
		write(SVELTEKIT_TSCONFIG_PATH, '{}');

		expect(await sveltekit_tsconfig_is_unsynced(dir)).toBe(false);
	});

	test('ignores a tsconfig that does not extend `$app/tsconfig`', async () => {
		write('tsconfig.json', '{"extends": "@tsconfig/svelte"}');

		expect(await sveltekit_tsconfig_is_unsynced(dir)).toBe(false);
	});

	test('a missing or unparseable tsconfig is not unsynced', async () => {
		expect(await sveltekit_tsconfig_is_unsynced(dir)).toBe(false);

		write('tsconfig.json', 'not json {');
		expect(await sveltekit_tsconfig_is_unsynced(dir)).toBe(false);
	});
});
