import { describe, test, assert, vi, beforeEach, afterEach } from 'vitest';
import { readFile } from 'node:fs/promises';
import { join } from 'node:path';

import { task as changeset_task, type Args } from '#lib/changeset.task.ts';
import { fs, type FsDir } from '#lib/fs.ts';
import { TaskError } from '#lib/task.ts';
import type { SpawnResult } from '#lib/spawn.ts';

import { create_mock_task_context } from './test_helpers.ts';
import { get_mock_spawn } from './spawn_mocks.ts';
import { answer_git_call, git_ops, install_mock_git } from './git_mocks.ts';

vi.mock('#lib/spawn.ts', async () =>
	(await import('./git_mocks.ts')).create_mock_git_spawn_module()
);

vi.mock('#lib/package_json.ts', async (import_original) => {
	const actual = await import_original<typeof import('#lib/package_json.ts')>();
	return { ...actual, package_json_load: vi.fn(async () => ({ name: 'pkg', private: false })) };
});

vi.mock('#lib/sveltekit_helpers.ts', () => ({
	has_sveltekit_library: vi.fn(async () => ({ ok: true }))
}));

const OK: SpawnResult = { kind: 'exited', code: 0 };

let dir: FsDir & AsyncDisposable;

const to_context = (args: Partial<Args> = {}) =>
	create_mock_task_context<Args>({
		_: ['the message'],
		minor: false,
		major: false,
		dir: dir.path,
		changelog: '@changesets/changelog-git',
		dep: false,
		origin: 'origin',
		changeset_cli: 'changeset',
		...args
	});

const run_changeset = (args?: Partial<Args>) => changeset_task.run(to_context(args));

const to_task_error = async (args?: Partial<Args>): Promise<TaskError> => {
	let error: unknown;
	try {
		await run_changeset(args);
	} catch (e) {
		error = e;
	}
	assert.instanceOf(error, TaskError);
	return error;
};

/**
 * Fails the changeset CLI call `failing` names with code `3`, and every `git add` when `add_code`
 * is given, while `changeset init` writes a config as the real one does.
 */
const install_cli_mocks = async (
	failing: ReadonlyArray<string> | null,
	add_code = 0,
	init_access = 'restricted'
): Promise<void> => {
	const mock_spawn = await get_mock_spawn();
	mock_spawn.reset();
	install_mock_git(mock_spawn);
	mock_spawn.result = async (call) => {
		if (call.name === 'git') return answer_git_call(call);
		if (failing && failing.join(' ') === call.args.join(' ')) return { kind: 'exited', code: 3 };
		if (call.args[0] === 'init')
			await dir.write('config.json', JSON.stringify({ access: init_access }));
		if (call.args[0] === 'add') await dir.write('new-changeset.md', '---\n---\n');
		return OK;
	};
	git_ops.git_command.mockImplementation(async (_name, args: Array<string>) =>
		args[0] === 'add' ? { kind: 'exited', code: add_code } : OK
	);
};

// the commit exits with `code`, while `changeset add --empty` writes the new changeset file
const install_mocks = async (code: number): Promise<void> => {
	const mock_spawn = await get_mock_spawn();
	mock_spawn.reset();
	install_mock_git(mock_spawn);
	mock_spawn.result = async (call) => {
		if (call.name === 'git') return answer_git_call(call);
		await dir.write('new-changeset.md', '---\n---\n');
		return OK;
	};
	git_ops.git_command.mockImplementation(async (_name, args: Array<string>) =>
		args[0] === 'commit' ? { kind: 'exited', code } : OK
	);
};

describe('gro changeset with a message', () => {
	beforeEach(async () => {
		vi.clearAllMocks();
		dir = await fs.temp_dir('gro-changeset-task-');
		await dir.write('config.json', '{}');
	});

	afterEach(async () => {
		await dir[Symbol.asyncDispose]();
	});

	test('pushes after a commit that succeeded', async () => {
		await install_mocks(0);
		await run_changeset();
		assert.strictEqual(git_ops.git_push_to_create.mock.calls.length, 1);
	});

	test('a failed commit throws and pushes nothing', async () => {
		await install_mocks(1);
		let error: unknown;
		try {
			await run_changeset();
		} catch (e) {
			error = e;
		}
		assert.instanceOf(error, TaskError);
		assert.include(error.message, 'nothing was pushed');
		assert.strictEqual(git_ops.git_push_to_create.mock.calls.length, 0);
	});

	test('a fully staged workspace commits and pushes', async () => {
		await install_mocks(0);
		git_ops.git_check_clean_workspace.mockResolvedValueOnce(null);
		const ctx = to_context();
		await changeset_task.run(ctx);
		assert.strictEqual(git_ops.git_push_to_create.mock.calls.length, 1);
		assert.ok(
			!vi.mocked(ctx.log.info).mock.calls.some(([line]) => String(line).includes('skipping')),
			'expected no skip message'
		);
	});

	test('a workspace that is not fully staged skips the commit and push and says so', async () => {
		await install_mocks(0);
		git_ops.git_check_clean_workspace.mockResolvedValueOnce('dirty');
		const ctx = to_context();
		await changeset_task.run(ctx);
		assert.strictEqual(git_ops.git_push_to_create.mock.calls.length, 0);
		assert.ok(
			!git_ops.git_command.mock.calls.some(([, args]) => args?.[0] === 'commit'),
			'expected no commit'
		);
		const [line] = vi.mocked(ctx.log.info).mock.calls.at(-1)!;
		assert.include(String(line), 'skipping the commit and push');
		assert.include(String(line), 'unstaged');
	});

	test('a failed `changeset add --empty` is a TaskError', async () => {
		await install_cli_mocks(['add', '--empty']);
		const error = await to_task_error();
		assert.include(error.message, '`changeset add --empty` failed: code 3');
	});

	test('a failed `git add` of the new changeset is a TaskError and pushes nothing', async () => {
		await install_cli_mocks(null, 1);
		const error = await to_task_error();
		assert.include(error.message, '`git add --');
		assert.include(error.message, 'failed: code 1');
		assert.strictEqual(git_ops.git_push_to_create.mock.calls.length, 0);
	});
});

describe('gro changeset with no message', () => {
	beforeEach(async () => {
		vi.clearAllMocks();
		dir = await fs.temp_dir('gro-changeset-task-');
		await dir.write('config.json', '{}');
	});

	afterEach(async () => {
		await dir[Symbol.asyncDispose]();
	});

	test('a failed interactive `changeset` is a TaskError', async () => {
		await install_cli_mocks([]);
		const error = await to_task_error({ _: [] });
		assert.include(error.message, '`changeset` failed: code 3');
	});

	test('a failed `git add` after the prompt is a TaskError', async () => {
		await install_cli_mocks(null, 1);
		const error = await to_task_error({ _: [] });
		assert.include(error.message, 'failed: code 1');
	});
});

describe('gro changeset initing', () => {
	beforeEach(async () => {
		vi.clearAllMocks();
		dir = await fs.temp_dir('gro-changeset-task-');
	});

	afterEach(async () => {
		await dir[Symbol.asyncDispose]();
	});

	test('a failed `changeset init` is a TaskError', async () => {
		await install_cli_mocks(['init']);
		const error = await to_task_error({ _: [] });
		assert.include(error.message, '`changeset init` failed: code 3');
	});

	const read_config = async (): Promise<Record<string, unknown>> =>
		JSON.parse(await readFile(join(dir.path, 'config.json'), 'utf8'));

	test('restricted access writes the changelog and stays restricted', async () => {
		await install_cli_mocks(null);
		await run_changeset({ _: [], access: 'restricted', changelog: 'my-changelog' });
		assert.deepEqual(await read_config(), { access: 'restricted', changelog: 'my-changelog' });
	});

	test('restricted access overrides a public default from init', async () => {
		await install_cli_mocks(null, 0, 'public');
		await run_changeset({ _: [], access: 'restricted', changelog: 'my-changelog' });
		assert.deepEqual(await read_config(), { access: 'restricted', changelog: 'my-changelog' });
	});

	test('public access writes both', async () => {
		await install_cli_mocks(null);
		await run_changeset({ _: [], access: 'public', changelog: 'my-changelog' });
		assert.deepEqual(await read_config(), { access: 'public', changelog: 'my-changelog' });
	});
});
