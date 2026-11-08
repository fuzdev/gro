import { describe, test, expect, vi, afterEach } from 'vitest';

import { task } from '#lib/test.task.ts';
import { SpawnError } from '#lib/spawn_error.ts';
import { is_task_error } from '#lib/task.ts';
import { create_mock_task_context } from './test_helpers.ts';
import { get_mock_spawn } from './spawn_mocks.ts';

vi.mock('#lib/spawn.ts', async () => (await import('./spawn_mocks.ts')).create_mock_spawn_module());

vi.mock('#lib/package_json.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/package_json.ts')>()),
	package_json_load: vi.fn(async () => ({
		name: 'a',
		version: '0',
		devDependencies: { vitest: '4' }
	}))
}));

const original_argv = process.argv;

afterEach(async () => {
	process.argv = original_argv;
	vi.clearAllMocks();
	(await get_mock_spawn()).reset();
});

// the vitest args after the ones the test task always passes
const run_test = async (cli: Array<string>): Promise<Array<string>> => {
	process.argv = ['node', 'gro', ...cli];
	await task.run(create_mock_task_context({ _: ['.test.'], dir: 'src', fail_without_tests: true }));
	const { calls } = await get_mock_spawn();
	expect(calls).toHaveLength(1);
	expect(calls[0]!.program_options).toEqual({ pass_env: 'all' });
	const vitest_args = [...calls[0]!.args];
	expect(vitest_args.slice(0, 4)).toEqual(['run', '.test.', '--dir', 'src']);
	return vitest_args.slice(4);
};

describe('test task with a missing or broken vitest', () => {
	const CTX_ARGS = { _: ['.test.'], dir: 'src', fail_without_tests: true };

	test('a missing vitest says so', async () => {
		(await get_mock_spawn()).available = () => false;
		process.argv = ['node', 'gro', 'test'];
		await expect(task.run(create_mock_task_context(CTX_ARGS))).rejects.toThrow(
			'vitest is a dependency but not installed; run `npm i`?'
		);
	});

	test("a vitest that can't run is a `TaskError`", async () => {
		const spawn_error = new SpawnError('not_executable', 'vitest', 'not_executable: the message');
		(await get_mock_spawn()).find_error = () => spawn_error;
		process.argv = ['node', 'gro', 'test'];
		await expect(task.run(create_mock_task_context(CTX_ARGS))).rejects.toSatisfy(
			(error) => is_task_error(error) && error.message === spawn_error.message
		);
	});
});

describe('test task forwarded args', () => {
	test('implicit: the first section verbatim when the CLI invoked `test`', async () => {
		expect(
			await run_test(['test', '--', '--no-cache', '--testTimeout', '0123', '-t', '-foo', '--x='])
		).toEqual(['--no-cache', '--testTimeout', '0123', '-t', '-foo', '--x=']);
	});

	test('implicit: only the first section', async () => {
		expect(await run_test(['test', '--', '--watch', '--', 'eslint', '--fix'])).toEqual(['--watch']);
	});

	test('implicit: none for a nested `test`', async () => {
		expect(await run_test(['check', '--', '--watch'])).toEqual([]);
		(await get_mock_spawn()).reset();
		expect(await run_test(['check', '--', 'gro', 'test', '--coverage'])).toEqual([]);
	});

	test('explicit `vitest` sections at any depth, after the implicit section', async () => {
		expect(await run_test(['test', '--', 'vitest', '--watch'])).toEqual(['--watch']);
		(await get_mock_spawn()).reset();
		expect(await run_test(['check', '--', 'vitest', '--bail', '01'])).toEqual(['--bail', '01']);
		(await get_mock_spawn()).reset();
		expect(await run_test(['test', '--', '--watch', '--', 'vitest', '--bail', '1'])).toEqual([
			'--watch',
			'--bail',
			'1'
		]);
	});
});
