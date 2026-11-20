import { test, assert, vi, beforeEach } from 'vitest';
import type { PackageJson } from '@fuzdev/fuz_util/package_json.ts';

import { run_typecheck, task } from '#lib/typecheck.task.ts';
import type { ProcessOutput } from '#lib/stdout.ts';
import { fs } from '#lib/fs.ts';
import { create_fs_paths } from '#lib/fs_paths.ts';
import { SVELTEKIT_DEP_NAME } from '#lib/constants.ts';

import { get_mock_spawn } from './spawn_mocks.ts';
import { create_mock_builtin_context, create_mock_logger } from './test_helpers.ts';
import { to_fs_paths_options } from './fs_test_helpers.ts';

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

const run_with = async (package_json: Partial<PackageJson>): Promise<number> => {
	const { package_json_load } = vi.mocked(await import('#lib/package_json.ts'));
	// tsc is declared and found, so it's what runs
	package_json_load.mockResolvedValue({
		name: 'a',
		version: '0',
		...package_json,
		devDependencies: { typescript: '6', ...package_json.devDependencies }
	});
	const mock_spawn = await get_mock_spawn();
	mock_spawn.available = (name) => name === 'node_modules/.bin/tsc';
	await task.run(
		await create_mock_builtin_context(task, { path_replacement: '.', cwd: process.cwd() })
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

test('syncs with the SvelteKit CLI the host grants, by its role', async () => {
	const { package_json_load } = vi.mocked(await import('#lib/package_json.ts'));
	package_json_load.mockResolvedValue({
		name: 'a',
		version: '0',
		devDependencies: { [SVELTEKIT_DEP_NAME]: '3' }
	});
	const find = vi.fn((_role: string) => null);
	await run_typecheck(
		{ path_replacement: '.', cwd: '/p' },
		{ raw_rest_args: [] },
		{
			fs_paths: create_fs_paths(fs, to_fs_paths_options()),
			programs: { find },
			stdout: {} as ProcessOutput,
			stderr: {} as ProcessOutput,
			log: create_mock_logger()
		}
	).catch(() => {});
	const { sveltekit_sync } = vi.mocked(await import('#lib/sveltekit_helpers.ts'));
	find.mockClear();
	sveltekit_sync.mock.calls[0]![0].find('svelte-kit');
	assert.deepEqual(find.mock.calls, [['sveltekit_cli']]);
});
