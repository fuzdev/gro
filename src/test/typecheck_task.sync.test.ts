import { test, assert, vi, beforeEach } from 'vitest';
import type { PackageJson } from '@fuzdev/fuz_util/package_json.ts';

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

const run_with = async (package_json: Partial<PackageJson>): Promise<number> => {
	const { package_json_load } = vi.mocked(await import('#lib/package_json.ts'));
	package_json_load.mockResolvedValueOnce({ name: 'a', version: '0', ...package_json });
	const mock_spawn = await get_mock_spawn();
	mock_spawn.available = (name) => name === 'node_modules/.bin/tsc';
	await task.run(
		create_mock_task_context({
			svelte_check_cli: 'svelte-check',
			typescript_cli: 'tsc',
			path_replacement: '.',
			cwd: process.cwd()
		})
	);
	const { sveltekit_sync } = vi.mocked(await import('#lib/sveltekit_helpers.ts'));
	return sveltekit_sync.mock.calls.length;
};

beforeEach(async () => {
	vi.clearAllMocks();
	(await get_mock_spawn()).reset();
});

test('syncs before typechecking a project with SvelteKit', async () => {
	assert.strictEqual(await run_with({ devDependencies: { [SVELTEKIT_DEP_NAME]: '3' } }), 1);
});

test('does not sync a project without SvelteKit, even with the CLI around', async () => {
	assert.strictEqual(await run_with({ devDependencies: { typescript: '6' } }), 0);
});

test('does not sync a project only peered on SvelteKit', async () => {
	assert.strictEqual(await run_with({ peerDependencies: { [SVELTEKIT_DEP_NAME]: '3' } }), 0);
});
