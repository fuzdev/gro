import { describe, test, assert, vi, beforeEach } from 'vitest';

import { task as publish_task, type Args } from '#lib/publish.task.ts';
import { TaskError, type TaskContext } from '#lib/task.ts';
import { process as process_host } from '#lib/process.ts';

import { create_mock_task_context } from './test_helpers.ts';
import { get_mock_spawn } from './spawn_mocks.ts';
import { git_ops, install_mock_git } from './git_mocks.ts';

vi.mock('#lib/package_json.ts', async (import_original) => {
	const actual = await import_original<typeof import('#lib/package_json.ts')>();
	return {
		...actual,
		package_json_load: vi.fn()
	};
});

vi.mock('#lib/spawn.ts', async () =>
	(await import('./git_mocks.ts')).create_mock_git_spawn_module()
);

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
			force_build: false,
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

		const mock_spawn = await get_mock_spawn();
		mock_spawn.reset();
		install_mock_git(mock_spawn);
		mock_spawn.available = (name) => name === 'changeset' || name === 'git'; // on `PATH`, not installed locally

		const { has_sveltekit_library } = vi.mocked(await import('#lib/sveltekit_helpers.ts'));
		has_sveltekit_library.mockResolvedValue({ ok: true });

		git_ops.git_command.mockResolvedValue({ kind: 'exited', code: 0 });
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
		assert.deepEqual(install_with_cache_healing_or_throw.mock.calls[0]![2], {
			log: ctx.log,
			stderr: process_host.stderr,
			context: 'after version bump'
		});
		assert.strictEqual(sveltekit_sync.mock.calls.length, 1);
		assert.deepEqual(sveltekit_sync.mock.calls[0], [await get_mock_spawn(), { pm_cli: 'npm' }]);

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

	test('fails loud when the final git push is rejected', async () => {
		const spawn = git_ops.git_command;
		spawn.mockImplementation(async (_command, args) =>
			args?.[0] === 'push' ? { kind: 'exited', code: 1 } : { kind: 'exited', code: 0 }
		);
		const ctx = create_mock_publish_task_context();

		let error;
		try {
			await publish_task.run(ctx);
		} catch (e) {
			error = e;
		}

		assert(error instanceof TaskError, 'expected a rejected push to throw a TaskError');
		assert.include(error.message, `tag v${VERSION_AFTER} are local only`);
		assert.include(error.message, 'already published to the registry');
		assert.include(error.message, '`git push --follow-tags origin main`');
		assert.deepEqual(spawn.mock.calls.at(-1)!.slice(0, 2), [
			'git',
			['push', '--follow-tags', '--end-of-options', 'origin', 'main']
		]);
	});

	test('the release push and its recovery message honor --origin and --branch', async () => {
		const spawn = git_ops.git_command;
		spawn.mockImplementation(async (_command, args) =>
			args?.[0] === 'push' ? { kind: 'exited', code: 1 } : { kind: 'exited', code: 0 }
		);
		const ctx = create_mock_publish_task_context({ origin: 'upstream', branch: 'release' });

		let error;
		try {
			await publish_task.run(ctx);
		} catch (e) {
			error = e;
		}

		assert(error instanceof TaskError, 'expected a rejected push to throw a TaskError');
		assert.include(error.message, '`git push --follow-tags upstream release`');
		assert.deepEqual(spawn.mock.calls.at(-1)!.slice(0, 2), [
			'git',
			['push', '--follow-tags', '--end-of-options', 'upstream', 'release']
		]);
	});

	test('forwards force_build to the build', async () => {
		const ctx = create_mock_publish_task_context({
			build: true,
			'no-build': false,
			force_build: true
		});

		await publish_task.run(ctx);

		assert.deepEqual(
			vi.mocked(ctx.invoke_task).mock.calls.find(([name]) => name === 'build'),
			['build', { sync: false, install: false, force_build: true }]
		);
	});

	test('fails loud when the release commit fails, without pushing', async () => {
		const spawn = git_ops.git_command;
		spawn.mockImplementation(async (_command, args) =>
			args?.[0] === 'commit' ? { kind: 'exited', code: 1 } : { kind: 'exited', code: 0 }
		);
		const ctx = create_mock_publish_task_context();

		let error;
		try {
			await publish_task.run(ctx);
		} catch (e) {
			error = e;
		}

		assert(error instanceof TaskError, 'expected a failed commit to throw a TaskError');
		assert.include(error.message, 'the release commit failed');
		assert.include(error.message, `v${VERSION_AFTER} is already published to the registry`);
		assert.ok(!spawn.mock.calls.some(([, args]) => args?.[0] === 'push'), 'expected no push');
	});

	test('a failed `changeset version` is a TaskError, before anything is committed', async () => {
		(await get_mock_spawn()).result = ({ args }) =>
			args[0] === 'version' ? { kind: 'exited', code: 1 } : { kind: 'exited', code: 0 };
		const ctx = create_mock_publish_task_context();

		let error;
		try {
			await publish_task.run(ctx);
		} catch (e) {
			error = e;
		}

		assert(error instanceof TaskError, 'expected a failed version bump to throw a TaskError');
		assert.include(error.message, 'changeset version failed: no commits were made');
		const { calls } = await get_mock_spawn();
		assert.ok(
			!calls.some(({ name, args }) => name === 'git' && args[0] === 'commit'),
			'expected no commit'
		);
	});

	const to_dry_error = async (package_json: object): Promise<unknown> => {
		const { package_json_load } = vi.mocked(await import('#lib/package_json.ts'));
		package_json_load.mockReset();
		package_json_load.mockResolvedValue(package_json as never);
		try {
			await publish_task.run(create_mock_publish_task_context({ dry: true }));
		} catch (e) {
			return e;
		}
		return undefined;
	};

	test('--dry refuses a package with no parseable repository url', async () => {
		const error = await to_dry_error({ name: 'a', version: '1.0.0' });
		assert(error instanceof TaskError, 'expected a TaskError');
		assert.include(error.message, 'must contain a repo url');
	});

	test('--dry refuses a package with no version', async () => {
		const error = await to_dry_error({
			name: 'a',
			repository: 'https://github.com/fuzdev/test_pkg'
		});
		assert(error instanceof TaskError, 'expected a TaskError');
		assert.include(error.message, 'Failed to find package.json version');
	});

	test('--dry passes a package a real publish would accept', async () => {
		assert.strictEqual(await to_dry_error(to_package_json(VERSION_BEFORE)), undefined);
	});
});
