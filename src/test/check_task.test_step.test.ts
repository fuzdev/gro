import { describe, test, assert, vi, beforeEach } from 'vitest';
import type { PackageJson } from '@fuzdev/fuz_util/package_json.ts';

import { run_check, type Args, type CheckDeps } from '#lib/check.task.ts';
import { is_task_error } from '#lib/task_error.ts';
import type { FsSearchResult } from '#lib/fs_search.ts';
import { FsError } from '#lib/fs_error.ts';

import { create_mock_logger } from './test_helpers.ts';
import { create_mock_fs_paths } from './fs_mocks.ts';
import { create_mock_git_program, git_ops } from './git_mocks.ts';

vi.mock('#lib/package_json.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/package_json.ts')>()),
	project_has_dependency: vi.fn()
}));

vi.mock('#lib/vitest_helpers.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/vitest_helpers.ts')>()),
	find_test_files: vi.fn(async () => [])
}));

const load_mock = async () =>
	vi.mocked(await import('#lib/package_json.ts')).project_has_dependency;
const find_mock = async () => vi.mocked(await import('#lib/vitest_helpers.ts')).find_test_files;

const declare = async (package_json: Partial<PackageJson>): Promise<void> => {
	const { package_json_has_dependency } = await import('#lib/package_json.ts');
	const declared = { name: 'a', version: '0', ...package_json };
	(await load_mock()).mockImplementation((_fs_paths, dep_name) =>
		Promise.resolve(package_json_has_dependency(dep_name, declared))
	);
};
const DECLARED = { devDependencies: { vitest: '5' } };

const to_test_files = (...paths: Array<string>): Array<FsSearchResult> =>
	paths.map((path) => ({
		path,
		id: `/project/src/${path}`,
		is_directory: false,
		kind: 'file' as const
	}));

const DEFAULT_ARGS = {
	typecheck: false,
	test: true,
	gen: false,
	format: false,
	package_json: false,
	lint: false,
	build: false,
	lockfile: false,
	install: false,
	workspace: false
} as unknown as Args;

/**
 * A check run over plain objects, its project `/project` and git answered from the mock table.
 */
interface CheckRun {
	args: Args;
	raw_rest_args: ReadonlyArray<string>;
	deps: CheckDeps;
}

const create_ctx = (
	args: Partial<Args> = {},
	raw_rest_args: ReadonlyArray<string> = []
): CheckRun => ({
	args: { ...DEFAULT_ARGS, ...args },
	raw_rest_args,
	deps: {
		fs_paths: create_mock_fs_paths(),
		env: { get: () => undefined },
		programs: { find: () => create_mock_git_program() },
		invoke_task: vi.fn(),
		log: create_mock_logger()
	}
});

const run = (ctx: CheckRun): Promise<void> =>
	run_check(ctx.args, { map_package_json: null, raw_rest_args: ctx.raw_rest_args }, ctx.deps);

const get_task_error = async (ctx: CheckRun): Promise<string> => {
	try {
		await run(ctx);
	} catch (error) {
		assert(is_task_error(error));
		return error.message;
	}
	assert.fail('expected a TaskError');
};

const skip_log_calls = (ctx: CheckRun): Array<string> =>
	vi
		.mocked(ctx.deps.log.info)
		.mock.calls.map((c) => String(c[0]))
		.filter((m) => m.includes('skipping tests'));

const test_invocations = (ctx: CheckRun): number =>
	vi.mocked(ctx.deps.invoke_task).mock.calls.filter((c) => c[0] === 'test').length;

describe('gro check test step', () => {
	beforeEach(async () => {
		vi.clearAllMocks();
		(await find_mock()).mockResolvedValue([]);
	});

	test('runs the test task with vitest declared, with no search', async () => {
		await declare(DECLARED);
		const ctx = create_ctx();
		await run(ctx);
		assert.strictEqual(test_invocations(ctx), 1);
		assert.strictEqual((await find_mock()).mock.calls.length, 0);
		assert.deepEqual(skip_log_calls(ctx), []);
	});

	test("turns the test task's sync off after the typecheck synced, and on without it", async () => {
		await declare(DECLARED);
		const test_args = async (args: Partial<Args>) => {
			const ctx = create_ctx(args);
			await run(ctx);
			return vi.mocked(ctx.deps.invoke_task).mock.calls.find((c) => c[0] === 'test')?.[1];
		};
		assert.deepEqual(await test_args({ typecheck: true }), { sync: false });
		assert.deepEqual(await test_args({}), {});
	});

	test('reads package.json with the task log, so a malformed one is reported', async () => {
		await declare(DECLARED);
		const ctx = create_ctx();
		await run(ctx);
		assert.strictEqual((await load_mock()).mock.calls[0]![2], ctx.deps.log);
	});

	test('searches the same directory `gro test` defaults to', async () => {
		await declare({});
		const ctx = create_ctx();
		await run(ctx);
		const calls = (await find_mock()).mock.calls;
		assert.strictEqual(calls.length, 1);
		assert.strictEqual(calls[0]![0], ctx.deps.fs_paths);
		assert.strictEqual(calls[0]![1], '/project/src/');
	});

	test('undeclared with no test files skips with a log line', async () => {
		await declare({});
		const ctx = create_ctx();
		await run(ctx);
		assert.strictEqual(test_invocations(ctx), 0);
		assert.deepEqual(skip_log_calls(ctx), [
			'vitest is not declared and no test files were found; skipping tests'
		]);
	});

	test('peer-only counts as undeclared', async () => {
		await declare({ peerDependencies: { vitest: '5' } });
		const ctx = create_ctx();
		await run(ctx);
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
		await run(ctx);
		assert.strictEqual(test_invocations(ctx), 0);
		assert.strictEqual((await load_mock()).mock.calls.length, 0);
		assert.strictEqual((await find_mock()).mock.calls.length, 0);
		assert.deepEqual(skip_log_calls(ctx), []);
	});

	test('--workspace skips an undeclared project the same way', async () => {
		await declare({});
		git_ops.git_check_clean_workspace.mockResolvedValue(null);
		const ctx = create_ctx({ workspace: true });
		await run(ctx);
		assert.strictEqual(test_invocations(ctx), 0);
		assert.strictEqual(skip_log_calls(ctx).length, 1);
	});

	test('--workspace fails an undeclared project with test files', async () => {
		await declare({});
		(await find_mock()).mockResolvedValue(to_test_files('a.test.ts'));
		git_ops.git_check_clean_workspace.mockResolvedValue(null);
		assert.include(await get_task_error(create_ctx({ workspace: true })), 'found 1 test file ');
	});

	test('a forwarded `gro test` section runs the test task, with no search', async () => {
		await declare({});
		const ctx = create_ctx({}, ['--', 'gro', 'test', '--dir', 'x']);
		await run(ctx);
		assert.strictEqual(test_invocations(ctx), 1);
		assert.strictEqual((await find_mock()).mock.calls.length, 0);
	});

	test('args forwarded to vitest run the test task, an empty section does not', async () => {
		await declare({});
		const ctx = create_ctx({}, ['--', 'vitest', '--dir', 'x']);
		await run(ctx);
		assert.strictEqual(test_invocations(ctx), 1);
		const empty_ctx = create_ctx({}, ['--', 'vitest']);
		await run(empty_ctx);
		assert.strictEqual(test_invocations(empty_ctx), 0);
		assert.strictEqual(skip_log_calls(empty_ctx).length, 1);
	});

	test('a forwarded section for another task still searches', async () => {
		await declare({});
		const ctx = create_ctx({}, ['--', 'gro', 'typecheck', '--a']);
		await run(ctx);
		assert.strictEqual(test_invocations(ctx), 0);
		assert.strictEqual((await find_mock()).mock.calls.length, 1);
	});

	test('a failed search is a `TaskError` naming the directory', async () => {
		await declare({});
		const fs_error = new FsError('permission_denied', '/project/src/locked');
		(await find_mock()).mockRejectedValue(fs_error);
		const ctx = create_ctx();
		try {
			await run(ctx);
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
			await run(create_ctx());
			assert.fail('expected the error');
		} catch (error) {
			assert.strictEqual(error, unexpected);
		}
	});
});
