import { describe, test, expect, vi, afterEach } from 'vitest';

import { task } from '#lib/lint.task.ts';
import { create_mock_task_context } from './test_helpers.ts';
import { get_mock_spawn } from './spawn_mocks.ts';
import { set_mock_process_args } from './process_mocks.ts';

vi.mock('#lib/process.ts', async () =>
	(await import('./process_mocks.ts')).create_mock_process_module()
);

vi.mock('#lib/spawn.ts', async () => (await import('./spawn_mocks.ts')).create_mock_spawn_module());

afterEach(async () => {
	set_mock_process_args([]);
	(await get_mock_spawn()).reset();
});

const run_lint = async (
	forwarded: Array<string>,
	_: Array<string> = []
): Promise<Array<string>> => {
	set_mock_process_args(['lint', '--', 'eslint', ...forwarded]);
	await task.run(create_mock_task_context({ _, eslint_cli: 'eslint' }));
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
