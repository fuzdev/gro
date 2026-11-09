import { test, assert, vi, beforeEach } from 'vitest';

import { task } from '#lib/typecheck.task.ts';
import { SVELTEKIT_DEP_NAME } from '#lib/constants.ts';

import { get_mock_spawn } from './spawn_mocks.ts';
import { create_mock_task_context } from './test_helpers.ts';

vi.mock('#lib/spawn.ts', async () => (await import('./spawn_mocks.ts')).create_mock_spawn_module());

vi.mock('#lib/sveltekit_helpers.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/sveltekit_helpers.ts')>()),
	sveltekit_sync: vi.fn(async () => undefined)
}));

vi.mock('#lib/package_json.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/package_json.ts')>()),
	package_json_load: vi.fn()
}));

const ARGS = {
	svelte_check_cli: 'svelte-check',
	typescript_cli: 'tsc',
	path_replacement: '.',
	cwd: process.cwd()
};

beforeEach(async () => {
	vi.clearAllMocks();
	(await get_mock_spawn()).reset();
});

test('syncs first, naming the configured package manager, reporting a bad package.json', async () => {
	const { package_json_load } = vi.mocked(await import('#lib/package_json.ts'));
	package_json_load.mockResolvedValueOnce({
		name: 'a',
		version: '0',
		devDependencies: { [SVELTEKIT_DEP_NAME]: '3' }
	});
	const mock_spawn = await get_mock_spawn();
	mock_spawn.available = (name) => name === 'node_modules/.bin/tsc';
	const { sveltekit_sync } = vi.mocked(await import('#lib/sveltekit_helpers.ts'));
	let spawned_before_sync = -1;
	sveltekit_sync.mockImplementationOnce(async () => {
		spawned_before_sync = mock_spawn.calls.length;
	});

	const ctx = create_mock_task_context(ARGS, { pm_cli: 'pnpm' });
	await task.run(ctx);

	assert.strictEqual(spawned_before_sync, 0);
	assert.strictEqual(mock_spawn.calls.length, 1);
	assert.deepEqual(sveltekit_sync.mock.calls[0]![1], { pm_cli: 'pnpm' });
	assert.strictEqual(package_json_load.mock.calls[0]![4], ctx.log);
});
