import { describe, test, assert, vi, beforeEach } from 'vitest';
import type { PackageJson } from '@fuzdev/fuz_util/package_json.ts';

import { task as check_task, type Args } from '#lib/check.task.ts';
import { paths } from '#lib/paths.ts';
import { is_task_error, type TaskContext } from '#lib/task.ts';
import { fs } from '#lib/fs.ts';
import type { FsSearchResult } from '#lib/fs_search.ts';

import { create_mock_task_context } from './test_helpers.ts';
import { get_mock_spawn } from './spawn_mocks.ts';
import { git_ops, install_mock_git } from './git_mocks.ts';
import { set_mock_process_args } from './process_mocks.ts';
import { FsError } from '#lib/fs_error.ts';

vi.mock('#lib/process.ts', async () =>
	(await import('./process_mocks.ts')).create_mock_process_module()
);

vi.mock('#lib/spawn.ts', async () => (await import('./spawn_mocks.ts')).create_mock_spawn_module());

vi.mock('#lib/package_json.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/package_json.ts')>()),
	package_json_load: vi.fn()
}));

vi.mock('#lib/vitest_helpers.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/vitest_helpers.ts')>()),
	find_test_files: vi.fn(async () => [])
}));

const load_mock = async () => vi.mocked(await import('#lib/package_json.ts')).package_json_load;
const find_mock = async () => vi.mocked(await import('#lib/vitest_helpers.ts')).find_test_files;

const declare = async (package_json: Partial<PackageJson>): Promise<void> => {
	(await load_mock()).mockResolvedValue({ name: 'a', version: '0', ...package_json });
};
const DECLARED = { devDependencies: { vitest: '5' } };

const to_test_files = (...paths: Array<string>): Array<FsSearchResult> =>
	paths.map((path) => ({
		path,
		id: `/project/src/${path}`,
		is_directory: false,
		kind: 'file' as const
	}));

const create_ctx = (args: Partial<Args> = {}): TaskContext<Args> =>
	create_mock_task_context<Args>(args, {}, {
		typecheck: false,
		test: true,
		gen: false,
		format: false,
		package_json: false,
		lint: false,
		build: false,
		sync: false,
		install: false,
		workspace: false
	} as unknown as Args);

const get_task_error = async (ctx: TaskContext<Args>): Promise<string> => {
	try {
		await check_task.run(ctx);
	} catch (error) {
		assert(is_task_error(error));
		return error.message;
	}
	assert.fail('expected a TaskError');
};

const skip_log_calls = (ctx: TaskContext<Args>): Array<string> =>
	vi
		.mocked(ctx.log.info)
		.mock.calls.map((c) => String(c[0]))
		.filter((m) => m.includes('skipping tests'));

const test_invocations = (ctx: TaskContext<Args>): number =>
	vi.mocked(ctx.invoke_task).mock.calls.filter((c) => c[0] === 'test').length;

describe('gro check test step', () => {
	beforeEach(async () => {
		vi.clearAllMocks();
		(await get_mock_spawn()).reset();
		(await find_mock()).mockResolvedValue([]);
		set_mock_process_args(['check']);
	});

	test('runs the test task with vitest declared, with no search', async () => {
		await declare(DECLARED);
		const ctx = create_ctx();
		await check_task.run(ctx);
		assert.strictEqual(test_invocations(ctx), 1);
		assert.strictEqual((await find_mock()).mock.calls.length, 0);
		assert.deepEqual(skip_log_calls(ctx), []);
	});

	test('reads package.json with the task log, so a malformed one is reported', async () => {
		await declare(DECLARED);
		const ctx = create_ctx();
		await check_task.run(ctx);
		assert.strictEqual((await load_mock()).mock.calls[0]![4], ctx.log);
	});

	test('searches the same directory `gro test` defaults to', async () => {
		await declare({});
		await check_task.run(create_ctx());
		const calls = (await find_mock()).mock.calls;
		assert.strictEqual(calls.length, 1);
		assert.strictEqual(calls[0]![0].project, fs.project);
		assert.strictEqual(calls[0]![1], paths.source);
	});

	test('undeclared with no test files skips with a log line', async () => {
		await declare({});
		const ctx = create_ctx();
		await check_task.run(ctx);
		assert.strictEqual(test_invocations(ctx), 0);
		assert.deepEqual(skip_log_calls(ctx), [
			'vitest is not declared and no test files were found; skipping tests'
		]);
	});

	test('peer-only counts as undeclared', async () => {
		await declare({ peerDependencies: { vitest: '5' } });
		const ctx = create_ctx();
		await check_task.run(ctx);
		assert.strictEqual(test_invocations(ctx), 0);
		assert.strictEqual(skip_log_calls(ctx).length, 1);
	});

	test('undeclared with test files fails, naming them and the fix', async () => {
		await declare({});
		(await find_mock()).mockResolvedValue(to_test_files('b.test.ts', 'lib/a.test.ts'));
		const ctx = create_ctx();
		const message = await get_task_error(ctx);
		assert.include(message, "found 2 test files but vitest isn't declared in package.json");
		assert.include(message, 'b.test.ts');
		assert.include(message, 'lib/a.test.ts');
		assert.include(message, 'add vitest to devDependencies');
		assert.include(message, '--no-test');
		assert.strictEqual(test_invocations(ctx), 0);
		assert.deepEqual(skip_log_calls(ctx), []);
	});

	test('a long list is capped, with the count of the rest', async () => {
		await declare({});
		const names = Array.from({ length: 7 }, (_, i) => `f${i}.test.ts`);
		(await find_mock()).mockResolvedValue(to_test_files(...names));
		const message = await get_task_error(create_ctx());
		assert.include(message, 'found 7 test files');
		assert.include(message, 'f4.test.ts');
		assert.notInclude(message, 'f5.test.ts');
		assert.include(message, 'and 2 more');
	});

	test('--no-test skips without detection or a search', async () => {
		const ctx = create_ctx({ test: false });
		await check_task.run(ctx);
		assert.strictEqual(test_invocations(ctx), 0);
		assert.strictEqual((await load_mock()).mock.calls.length, 0);
		assert.strictEqual((await find_mock()).mock.calls.length, 0);
		assert.deepEqual(skip_log_calls(ctx), []);
	});

	test('--workspace skips an undeclared project the same way', async () => {
		await declare({});
		install_mock_git(await get_mock_spawn());
		git_ops.git_check_clean_workspace.mockResolvedValue(null);
		const ctx = create_ctx({ workspace: true });
		await check_task.run(ctx);
		assert.strictEqual(test_invocations(ctx), 0);
		assert.strictEqual(skip_log_calls(ctx).length, 1);
	});

	test('--workspace fails an undeclared project with test files', async () => {
		await declare({});
		(await find_mock()).mockResolvedValue(to_test_files('a.test.ts'));
		install_mock_git(await get_mock_spawn());
		git_ops.git_check_clean_workspace.mockResolvedValue(null);
		assert.include(await get_task_error(create_ctx({ workspace: true })), 'found 1 test file ');
	});

	test('a forwarded `gro test` section runs the test task, with no search', async () => {
		await declare({});
		set_mock_process_args(['check', '--', 'gro', 'test', '--dir', 'x']);
		const ctx = create_ctx();
		await check_task.run(ctx);
		assert.strictEqual(test_invocations(ctx), 1);
		assert.strictEqual((await find_mock()).mock.calls.length, 0);
	});

	test('args forwarded to vitest run the test task, an empty section does not', async () => {
		await declare({});
		set_mock_process_args(['check', '--', 'vitest', '--dir', 'x']);
		const ctx = create_ctx();
		await check_task.run(ctx);
		assert.strictEqual(test_invocations(ctx), 1);
		set_mock_process_args(['check', '--', 'vitest']);
		const empty_ctx = create_ctx();
		await check_task.run(empty_ctx);
		assert.strictEqual(test_invocations(empty_ctx), 0);
		assert.strictEqual(skip_log_calls(empty_ctx).length, 1);
	});

	test('a forwarded section for another task still searches', async () => {
		await declare({});
		set_mock_process_args(['check', '--', 'gro', 'typecheck', '--a']);
		const ctx = create_ctx();
		await check_task.run(ctx);
		assert.strictEqual(test_invocations(ctx), 0);
		assert.strictEqual((await find_mock()).mock.calls.length, 1);
	});

	test('a failed search is a `TaskError` naming the directory', async () => {
		await declare({});
		const fs_error = new FsError('permission_denied', '/project/src/locked');
		(await find_mock()).mockRejectedValue(fs_error);
		const ctx = create_ctx();
		try {
			await check_task.run(ctx);
			assert.fail('expected a TaskError');
		} catch (error) {
			assert(is_task_error(error));
			assert.include(error.message, 'failed to search src for test files');
			assert.include(error.message, 'permission_denied: /project/src/locked');
			assert.strictEqual(error.cause, fs_error);
		}
		assert.deepEqual(skip_log_calls(ctx), []);
	});

	test('a search failing other than on the fs is rethrown as it is', async () => {
		await declare({});
		const unexpected = Error('unexpected');
		(await find_mock()).mockRejectedValue(unexpected);
		try {
			await check_task.run(create_ctx());
			assert.fail('expected the error');
		} catch (error) {
			assert.strictEqual(error, unexpected);
		}
	});
});
