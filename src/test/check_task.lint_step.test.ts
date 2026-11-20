import { describe, test, assert, vi, beforeEach } from 'vitest';
import type { PackageJson } from '@fuzdev/fuz_util/package_json.ts';

import { run_check, type Args, type CheckDeps } from '#lib/check.task.ts';
import { is_task_error } from '#lib/task_error.ts';
import { FsError } from '#lib/fs_error.ts';

import { create_mock_logger } from './test_helpers.ts';
import { create_mock_fs_paths } from './fs_mocks.ts';

vi.mock('#lib/package_json.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/package_json.ts')>()),
	project_has_dependency: vi.fn()
}));

vi.mock('#lib/eslint_helpers.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/eslint_helpers.ts')>()),
	find_eslint_config: vi.fn(() => Promise.resolve(null))
}));

const declared_mock = async () =>
	vi.mocked(await import('#lib/package_json.ts')).project_has_dependency;
const config_mock = async () =>
	vi.mocked(await import('#lib/eslint_helpers.ts')).find_eslint_config;

const declare = async (package_json: Partial<PackageJson>): Promise<void> => {
	const { package_json_has_dependency } = await import('#lib/package_json.ts');
	const declared = { name: 'a', version: '0', ...package_json };
	(await declared_mock()).mockImplementation((_fs_paths, dep_name) =>
		Promise.resolve(package_json_has_dependency(dep_name, declared))
	);
};
const DECLARED = { devDependencies: { eslint: '9' } };

const NONE = {
	typecheck: false,
	test: false,
	gen: false,
	format: false,
	package_json: false,
	lint: true,
	build: false,
	sync: false,
	install: false,
	workspace: false
} as unknown as Args;

const create_deps = (): CheckDeps => ({
	fs_paths: create_mock_fs_paths(),
	env: { get: () => undefined },
	programs: { find: () => null },
	invoke_task: vi.fn(),
	log: create_mock_logger()
});

const run = (
	deps: CheckDeps,
	args: Partial<Args> = {},
	raw_rest_args: ReadonlyArray<string> = []
): Promise<void> =>
	run_check({ ...NONE, ...args }, { map_package_json: null, raw_rest_args }, deps);

const get_task_error = async (deps: CheckDeps): Promise<unknown> => {
	try {
		await run(deps);
	} catch (error) {
		return error;
	}
	return assert.fail('expected a TaskError');
};

const lint_invocations = (deps: CheckDeps): number =>
	vi.mocked(deps.invoke_task).mock.calls.filter((c) => c[0] === 'lint').length;

const skip_log_calls = (deps: CheckDeps): Array<string> =>
	vi
		.mocked(deps.log.info)
		.mock.calls.map((c) => String(c[0]))
		.filter((m) => m.includes('skipping lint'));

describe('gro check lint step', () => {
	beforeEach(async () => {
		vi.clearAllMocks();
		(await config_mock()).mockResolvedValue(null);
	});

	test('runs the lint task with eslint declared, looking for no config', async () => {
		await declare(DECLARED);
		const deps = create_deps();
		await run(deps);
		assert.strictEqual(lint_invocations(deps), 1);
		assert.strictEqual((await config_mock()).mock.calls.length, 0);
		assert.strictEqual((await declared_mock()).mock.calls[0]![2], deps.log);
	});

	test('undeclared with no ESLint config skips with a log line', async () => {
		await declare({});
		const deps = create_deps();
		await run(deps);
		assert.strictEqual(lint_invocations(deps), 0);
		assert.deepEqual(skip_log_calls(deps), [
			'eslint is not declared and the project root has no ESLint config; skipping lint'
		]);
		assert.strictEqual((await config_mock()).mock.calls[0]![0], deps.fs_paths);
	});

	test('peer-only counts as undeclared', async () => {
		await declare({ peerDependencies: { eslint: '9' } });
		const deps = create_deps();
		await run(deps);
		assert.strictEqual(lint_invocations(deps), 0);
		assert.strictEqual(skip_log_calls(deps).length, 1);
	});

	test('undeclared with an ESLint config fails, naming it and the fix', async () => {
		await declare({});
		(await config_mock()).mockResolvedValue('eslint.config.js');
		const deps = create_deps();
		const error = await get_task_error(deps);
		assert(is_task_error(error));
		assert.strictEqual(
			error.message,
			"found eslint.config.js but eslint isn't declared in package.json" +
				' - add eslint to devDependencies, remove eslint.config.js, or pass --no-lint'
		);
		assert.strictEqual(lint_invocations(deps), 0);
		assert.deepEqual(skip_log_calls(deps), []);
	});

	test('a failed config lookup is a `TaskError` with the fs error as its cause', async () => {
		await declare({});
		const fs_error = new FsError('permission_denied', '/project/eslint.config.js');
		(await config_mock()).mockRejectedValue(fs_error);
		const error = await get_task_error(create_deps());
		assert(is_task_error(error));
		assert.include(error.message, 'failed to look for an ESLint config');
		assert.strictEqual(error.cause, fs_error);
	});

	test('--no-lint skips without detection or a lookup', async () => {
		const deps = create_deps();
		await run(deps, { lint: false });
		assert.strictEqual(lint_invocations(deps), 0);
		assert.strictEqual((await declared_mock()).mock.calls.length, 0);
		assert.strictEqual((await config_mock()).mock.calls.length, 0);
		assert.deepEqual(skip_log_calls(deps), []);
	});

	test('a forwarded `gro lint` or eslint section runs the lint task, which decides', async () => {
		for (const forwarded of [
			['--', 'gro', 'lint', 'src'],
			['--', 'eslint', '--no-cache']
		]) {
			await declare({});
			const deps = create_deps();
			await run(deps, {}, forwarded);
			assert.strictEqual(lint_invocations(deps), 1, forwarded.join(' '));
			assert.strictEqual((await config_mock()).mock.calls.length, 0);
		}
	});

	test("an empty eslint section or another task's section still detects", async () => {
		for (const forwarded of [
			['--', 'eslint'],
			['--', 'gro', 'test', '--a']
		]) {
			await declare({});
			const deps = create_deps();
			await run(deps, {}, forwarded);
			assert.strictEqual(lint_invocations(deps), 0, forwarded.join(' '));
			assert.strictEqual(skip_log_calls(deps).length, 1);
		}
	});
});
