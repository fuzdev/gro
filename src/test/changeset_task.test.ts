import { describe, test, assert, vi, beforeEach, afterEach } from 'vitest';

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

const run_changeset = () =>
	changeset_task.run(
		create_mock_task_context<Args>({
			_: ['the message'],
			minor: false,
			major: false,
			dir: dir.path,
			changelog: '@changesets/changelog-git',
			dep: false,
			origin: 'origin',
			changeset_cli: 'changeset'
		})
	);

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
});
