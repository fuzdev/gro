import { describe, test, expect, vi, afterEach } from 'vitest';

import { task } from '#lib/test.task.ts';
import { spawn_cli } from '#lib/cli.ts';
import { create_mock_task_context } from './test_helpers.ts';

vi.mock('#lib/cli.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/cli.ts')>()),
	find_cli: vi.fn(async (name: string) => ({ kind: 'local', name, id: '/' + name })),
	spawn_cli: vi.fn(async () => ({ ok: true }))
}));

vi.mock('#lib/package_json.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/package_json.ts')>()),
	package_json_load: vi.fn(async () => ({
		name: 'a',
		version: '0',
		devDependencies: { vitest: '4' }
	}))
}));

const original_argv = process.argv;

afterEach(() => {
	process.argv = original_argv;
	vi.clearAllMocks();
});

// the vitest args after the ones the test task always passes
const run_test = async (cli: Array<string>): Promise<Array<string>> => {
	process.argv = ['node', 'gro', ...cli];
	await task.run(create_mock_task_context({ _: ['.test.'], dir: 'src', fail_without_tests: true }));
	expect(vi.mocked(spawn_cli)).toHaveBeenCalledTimes(1);
	const vitest_args = vi.mocked(spawn_cli).mock.calls[0]![1]!;
	expect(vitest_args.slice(0, 4)).toEqual(['run', '.test.', '--dir', 'src']);
	return vitest_args.slice(4);
};

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
		vi.clearAllMocks();
		expect(await run_test(['check', '--', 'gro', 'test', '--coverage'])).toEqual([]);
	});

	test('explicit `vitest` sections at any depth, after the implicit section', async () => {
		expect(await run_test(['test', '--', 'vitest', '--watch'])).toEqual(['--watch']);
		vi.clearAllMocks();
		expect(await run_test(['check', '--', 'vitest', '--bail', '01'])).toEqual(['--bail', '01']);
		vi.clearAllMocks();
		expect(await run_test(['test', '--', '--watch', '--', 'vitest', '--bail', '1'])).toEqual([
			'--watch',
			'--bail',
			'1'
		]);
	});
});
