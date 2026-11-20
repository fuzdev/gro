import { describe, test, expect, vi, afterEach } from 'vitest';

import { task } from '#lib/lint.task.ts';
import { create_mock_builtin_context } from './test_helpers.ts';
import { get_mock_spawn } from './spawn_mocks.ts';

vi.mock('#lib/spawn.ts', async () => (await import('./spawn_mocks.ts')).create_mock_spawn_module());

afterEach(async () => {
	(await get_mock_spawn()).reset();
});

const run_lint = async (
	forwarded: Array<string>,
	_: Array<string> = []
): Promise<Array<string>> => {
	await task.run(
		await create_mock_builtin_context(
			task,
			{ _, eslint_cli: 'eslint' },
			{ name: 'lint', cli_args: ['lint', '--', 'eslint', ...forwarded] }
		)
	);
	const { finds, calls } = await get_mock_spawn();
	expect(finds[0]).toBe('node_modules/.bin/eslint');
	expect(calls).toHaveLength(1);
	expect(calls[0]!.program_options).toEqual({ pass_env: 'all' });
	return [...calls[0]!.args];
};

describe('lint task forwarded args', () => {
	test('forwards the text verbatim after the paths and `--max-warnings 0`', async () => {
		expect(await run_lint(['--no-cache', '--rule', '-foo', '--x='], ['src'])).toEqual([
			'src',
			'--max-warnings',
			'0',
			'--no-cache',
			'--rule',
			'-foo',
			'--x='
		]);
	});

	test('a forwarded `--max-warnings` in any form drops the default', async () => {
		expect(await run_lint(['--max-warnings', '-1'])).toEqual(['--max-warnings', '-1']);
		(await get_mock_spawn()).reset();
		expect(await run_lint(['--max-warnings=10'])).toEqual(['--max-warnings=10']);
	});
});

test("`--eslint_cli` names the linter the host finds, the project's first", async () => {
	await task.run(
		await create_mock_builtin_context(task, { _: [], eslint_cli: 'oxlint' }, { name: 'lint' })
	);
	expect((await get_mock_spawn()).finds[0]).toBe('node_modules/.bin/oxlint');
});

test("a chosen `--eslint_cli` gets its own section, not ESLint's", async () => {
	(await get_mock_spawn()).reset();
	await task.run(
		await create_mock_builtin_context(
			task,
			{ _: [], eslint_cli: 'oxlint' },
			{ name: 'lint', cli_args: ['lint', '--', 'oxlint', '--x', '--', 'eslint', '--y'] }
		)
	);
	const { calls } = await get_mock_spawn();
	expect(calls).toHaveLength(1);
	expect(calls[0]!.name).toBe('node_modules/.bin/oxlint');
	expect(calls[0]!.args).toEqual(['--max-warnings', '0', '--x']);
});
