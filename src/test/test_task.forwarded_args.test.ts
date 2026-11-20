import { describe, test, expect, vi, afterEach } from 'vitest';

import * as s from '@fuzdev/fuz_schema/schema.ts';

import { Args, task } from '#lib/test.task.ts';
import { create_paths } from '#lib/paths.ts';
import { cwd } from '#lib/cwd.ts';
import { SpawnError } from '#lib/spawn_error.ts';
import { is_task_error } from '#lib/task_error.ts';
import { create_mock_builtin_context } from './test_helpers.ts';
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

afterEach(async () => {
	vi.clearAllMocks();
	(await get_mock_spawn()).reset();
});

// the vitest args after the ones the test task always passes
const run_test = async (cli: Array<string>): Promise<Array<string>> => {
	await task.run(
		await create_mock_builtin_context(
			task,
			{ _: ['.test.'], dir: 'src', fail_without_tests: true },
			{ name: 'test', cli_args: cli }
		)
	);
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
		await expect(task.run(await create_mock_builtin_context(task, CTX_ARGS))).rejects.toThrow(
			'vitest is a dependency but not installed; run `npm i`?'
		);
	});

	test("a vitest that can't run is a `TaskError`", async () => {
		const spawn_error = new SpawnError('not_executable', 'vitest');
		(await get_mock_spawn()).find_error = () => spawn_error;
		await expect(task.run(await create_mock_builtin_context(task, CTX_ARGS))).rejects.toSatisfy(
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

	test('implicit: none for a first section of a gro task', async () => {
		expect(await run_test(['test', '--', 'gro', 'test', '--coverage'])).toEqual([]);
	});

	test('implicit: what a caller passes, but for a section naming `vitest`, the explicit one', async () => {
		const run_given = async (implicit_rest_args: Array<string>): Promise<Array<string>> => {
			const ctx = await create_mock_builtin_context(task, {
				_: ['.test.'],
				dir: 'src',
				fail_without_tests: true
			});
			const mock_spawn = await get_mock_spawn();
			mock_spawn.reset();
			// the section `test` reads is its context's, whoever passed it
			await task.run({ ...ctx, implicit_rest_args });
			return mock_spawn.calls[0]!.args.slice(4);
		};
		expect(await run_given(['gro', '--x'])).toEqual(['gro', '--x']);
		expect(await run_given(['vitest', '--x'])).toEqual([]);
		expect(await run_given(['--x', 'vitest'])).toEqual(['--x', 'vitest']);
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

test("`dir` defaults to the working directory's `src/`", () => {
	expect(s.parse_or_throw(Args, {}).dir).toBe(create_paths(cwd()).source);
});

describe('test task help and version flags', () => {
	const run_cli = async (cli: Array<string>): Promise<unknown> => {
		try {
			await run_test(cli);
			return null;
		} catch (error) {
			return error;
		}
	};

	test('a help flag for vitest is refused, in its section or the implicit one', async () => {
		for (const cli of [
			['test', '--', 'vitest', '--help'],
			// a filter before it names no tool, the args all being Vitest's
			['test', '--', 'foo', '-h']
		]) {
			const error = await run_cli(cli);
			expect(is_task_error(error) && error.message).toMatch(
				/^`-?-h(elp)?` forwarded to vitest asks/
			);
		}
		expect((await get_mock_spawn()).calls).toHaveLength(0);
	});

	test('a version flag after `run` is left to vitest, which ignores it and runs the tests', async () => {
		expect(await run_test(['test', '--', 'vitest', '--version'])).toEqual(['--version']);
		(await get_mock_spawn()).reset();
		expect(await run_test(['test', '--', '-v'])).toEqual(['-v']);
	});
});
