import { describe, test, assert, vi, beforeEach } from 'vitest';

import { task as publish_task, type Args } from '#lib/publish.task.ts';
import type { TaskContext } from '#lib/task.ts';

import { create_mock_task_context } from './test_helpers.ts';

vi.mock('@fuzdev/fuz_util/git.js', async (import_original) => {
	const actual = await import_original<typeof import('@fuzdev/fuz_util/git.ts')>();
	return {
		...actual, // preserves GitBranch, GitOrigin, and other exports
		git_check_clean_workspace: vi.fn(),
		git_fetch: vi.fn(),
		git_checkout: vi.fn(),
		git_pull: vi.fn()
	};
});

vi.mock('@fuzdev/fuz_util/process.js', () => ({
	spawn: vi.fn()
}));

vi.mock('@fuzdev/fuz_util/fs.js', () => ({
	fs_exists: vi.fn()
}));

vi.mock('#lib/package_json.ts', async (import_original) => {
	const actual = await import_original<typeof import('#lib/package_json.ts')>();
	return {
		...actual,
		package_json_load: vi.fn()
	};
});

vi.mock('#lib/cli.ts', () => ({
	find_cli: vi.fn(),
	spawn_cli: vi.fn()
}));

vi.mock('#lib/sveltekit_helpers.ts', () => ({
	has_sveltekit_library: vi.fn(),
	sveltekit_sync: vi.fn()
}));

vi.mock('#lib/npm_install_helpers.ts', () => ({
	install_with_cache_healing_or_throw: vi.fn()
}));

vi.mock('#lib/changelog.ts', () => ({
	update_changelog: vi.fn()
}));

const create_mock_publish_task_context = (args: Partial<Args> = {}): TaskContext<Args> =>
	create_mock_task_context(
		args,
		{},
		{
			branch: 'main',
			origin: 'origin',
			changelog: 'CHANGELOG.md',
			preserve_changelog: true,
			optional: false,
			dry: false,
			check: false,
			'no-check': true,
			build: false,
			'no-build': true,
			pull: false,
			'no-pull': true,
			sync: true,
			'no-sync': false,
			install: true,
			'no-install': false,
			changeset_cli: 'changeset'
		}
	);

const VERSION_BEFORE = '1.0.0';
const VERSION_AFTER = '1.1.0';

const to_package_json = (version: string) => ({
	name: '@fuzdev/test_pkg',
	version,
	repository: 'https://github.com/fuzdev/test_pkg'
});

describe('publish_task version bump', () => {
	beforeEach(async () => {
		vi.clearAllMocks();

		const { package_json_load } = vi.mocked(await import('#lib/package_json.ts'));
		package_json_load
			.mockResolvedValueOnce(to_package_json(VERSION_BEFORE))
			.mockResolvedValueOnce(to_package_json(VERSION_AFTER));

		const { find_cli, spawn_cli } = vi.mocked(await import('#lib/cli.ts'));
		find_cli.mockResolvedValue({ name: 'changeset', id: '/bin/changeset', kind: 'global' });
		spawn_cli.mockResolvedValue({ kind: 'exited', ok: true, child: {} as any, code: 0 });

		const { has_sveltekit_library } = vi.mocked(await import('#lib/sveltekit_helpers.ts'));
		has_sveltekit_library.mockResolvedValue({ ok: true });

		const { fs_exists } = vi.mocked(await import('@fuzdev/fuz_util/fs.ts'));
		fs_exists.mockResolvedValue(true);
	});

	test('re-syncs SvelteKit after the post-bump install and before gen', async () => {
		const ctx = create_mock_publish_task_context();

		await publish_task.run(ctx);

		const { install_with_cache_healing_or_throw } = vi.mocked(
			await import('#lib/npm_install_helpers.ts')
		);
		const { sveltekit_sync } = vi.mocked(await import('#lib/sveltekit_helpers.ts'));
		const invoke_task = vi.mocked(ctx.invoke_task);

		assert.strictEqual(install_with_cache_healing_or_throw.mock.calls.length, 1);
		assert.deepEqual(install_with_cache_healing_or_throw.mock.calls[0]![1], {
			log: ctx.log,
			context: 'after version bump'
		});
		assert.strictEqual(sveltekit_sync.mock.calls.length, 1);
		assert.deepEqual(sveltekit_sync.mock.calls[0], [undefined, 'npm']);

		const gen_call_index = invoke_task.mock.calls.findIndex(([name]) => name === 'gen');
		assert.ok(gen_call_index !== -1, 'expected gen to run after the version bump');

		const install_order = install_with_cache_healing_or_throw.mock.invocationCallOrder[0]!;
		const sync_order = sveltekit_sync.mock.invocationCallOrder[0]!;
		const gen_order = invoke_task.mock.invocationCallOrder[gen_call_index]!;
		assert.ok(install_order < sync_order, 'SvelteKit must re-sync after the install');
		assert.ok(sync_order < gen_order, 'SvelteKit must re-sync before gen');
	});

	test('skips the post-bump install and re-sync with --no-install', async () => {
		const ctx = create_mock_publish_task_context({ install: false, 'no-install': true });

		await publish_task.run(ctx);

		const { install_with_cache_healing_or_throw } = vi.mocked(
			await import('#lib/npm_install_helpers.ts')
		);
		const { sveltekit_sync } = vi.mocked(await import('#lib/sveltekit_helpers.ts'));
		const invoke_task = vi.mocked(ctx.invoke_task);

		assert.strictEqual(install_with_cache_healing_or_throw.mock.calls.length, 0);
		assert.strictEqual(sveltekit_sync.mock.calls.length, 0);
		assert.ok(invoke_task.mock.calls.some(([name]) => name === 'gen'));
	});
});
