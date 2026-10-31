import { describe, test, expect, vi, afterEach } from 'vitest';

import { task } from '#lib/lint.task.ts';
import { find_cli, spawn_cli } from '#lib/cli.ts';
import { create_mock_task_context } from './test_helpers.ts';

vi.mock('#lib/cli.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/cli.ts')>()),
	find_cli: vi.fn(async (name: string) => ({ kind: 'local', name, id: '/' + name })),
	spawn_cli: vi.fn(async () => ({ ok: true }))
}));

const original_argv = process.argv;

afterEach(() => {
	process.argv = original_argv;
	vi.clearAllMocks();
});

const run_lint = async (
	forwarded: Array<string>,
	_: Array<string> = []
): Promise<Array<string>> => {
	process.argv = ['node', 'gro', 'lint', '--', 'eslint', ...forwarded];
	await task.run(create_mock_task_context({ _, eslint_cli: 'eslint' }));
	expect(vi.mocked(find_cli)).toHaveBeenCalledWith('eslint');
	expect(vi.mocked(spawn_cli)).toHaveBeenCalledTimes(1);
	return vi.mocked(spawn_cli).mock.calls[0]![1]!;
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
		vi.clearAllMocks();
		expect(await run_lint(['--max-warnings=10'])).toEqual(['--max-warnings=10']);
	});
});
