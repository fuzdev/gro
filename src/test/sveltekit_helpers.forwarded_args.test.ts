import { describe, test, expect, vi, afterEach } from 'vitest';
import { Logger } from '@fuzdev/fuz_util/log.ts';

import { run_svelte_package } from '#lib/sveltekit_helpers.ts';
import { spawn_cli, type Cli } from '#lib/cli.ts';
import { SVELTE_PACKAGE_DEP_NAME, SVELTEKIT_DEP_NAME } from '#lib/constants.ts';

vi.mock('#lib/cli.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/cli.ts')>()),
	spawn_cli: vi.fn(async () => undefined)
}));

const original_argv = process.argv;

afterEach(() => {
	process.argv = original_argv;
	vi.clearAllMocks();
});

describe('run_svelte_package forwarded args', () => {
	test('serializes a forwarded key after the configured options', async () => {
		process.argv = ['node', 'gro', 'build', '--', 'svelte-package', '--types'];
		const cli: Cli = { kind: 'local', name: 'svelte-package', id: '/svelte-package' };
		await run_svelte_package(
			{
				name: 'a',
				version: '0',
				devDependencies: { [SVELTEKIT_DEP_NAME]: '2', [SVELTE_PACKAGE_DEP_NAME]: '2' }
			},
			{ types: true, watch: true },
			cli,
			new Logger('test'),
			'npm'
		);
		expect(vi.mocked(spawn_cli)).toHaveBeenCalledTimes(1);
		expect(vi.mocked(spawn_cli).mock.calls[0]![1]).toEqual(['--watch', '--types']);
	});
});
