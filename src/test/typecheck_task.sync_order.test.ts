import { test, assert, vi, beforeEach } from 'vitest';

import { task } from '#lib/typecheck.task.ts';
import { SVELTEKIT_DEP_NAME } from '#lib/constants.ts';

import { get_mock_spawn } from './spawn_mocks.ts';
import { create_mock_builtin_context } from './test_helpers.ts';

vi.mock('#lib/spawn.ts', async () => (await import('./spawn_mocks.ts')).create_mock_spawn_module());

// detection reads through the mocked `package_json_load` whichever module loads first,
// as `package_json.ts` and `sveltekit_helpers.ts` import each other
vi.mock('#lib/sveltekit_helpers.ts', async (import_original) => {
	const original = await import_original<typeof import('#lib/sveltekit_helpers.ts')>();
	return {
		...original,
		sveltekit_sync: vi.fn(async () => undefined),
		project_has_sveltekit_app: vi.fn(
			async (fs_paths, log) =>
				original.has_sveltekit_app(
					await (await import('#lib/package_json.ts')).package_json_load(
						fs_paths,
						undefined,
						undefined,
						false,
						log
					)
				).ok
		)
	};
});

// `project_has_dependency` reads through the mocked `package_json_load`
vi.mock('#lib/package_json.ts', async (import_original) => {
	const original = await import_original<typeof import('#lib/package_json.ts')>();
	const package_json_load = vi.fn();
	return {
		...original,
		package_json_load,
		project_has_dependency: vi.fn(async (fs_paths, dep_name: string, log) =>
			original.package_json_has_dependency(
				dep_name,
				await package_json_load(fs_paths, undefined, undefined, false, log)
			)
		)
	};
});

// the project's `src/` has no Svelte files, so an undeclared svelte-check leaves tsc
vi.mock('#lib/svelte_helpers.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/svelte_helpers.ts')>()),
	find_svelte_files: vi.fn(() => Promise.resolve([]))
}));

const ARGS = {
	path_replacement: '.',
	cwd: process.cwd()
};

beforeEach(async () => {
	vi.clearAllMocks();
	(await get_mock_spawn()).reset();
});

test('syncs first, reporting a bad package.json', async () => {
	const { package_json_load } = vi.mocked(await import('#lib/package_json.ts'));
	package_json_load.mockResolvedValue({
		name: 'a',
		version: '0',
		devDependencies: { [SVELTEKIT_DEP_NAME]: '3', typescript: '6' }
	});
	const mock_spawn = await get_mock_spawn();
	mock_spawn.available = (name) => name === 'node_modules/.bin/tsc';
	const { sveltekit_sync } = vi.mocked(await import('#lib/sveltekit_helpers.ts'));
	let spawned_before_sync = -1;
	sveltekit_sync.mockImplementationOnce(async () => {
		spawned_before_sync = mock_spawn.calls.length;
	});

	const ctx = await create_mock_builtin_context(task, ARGS);
	await task.run(ctx);

	assert.strictEqual(spawned_before_sync, 0);
	assert.strictEqual(mock_spawn.calls.length, 1);
	assert.strictEqual(sveltekit_sync.mock.calls[0]!.length, 1);
	assert.strictEqual(package_json_load.mock.calls[0]![4], ctx.log);
});
