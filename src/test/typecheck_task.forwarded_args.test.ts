import { describe, test, expect, vi, afterEach } from 'vitest';

import { task } from '#lib/typecheck.task.ts';
import { find_cli, spawn_cli, spawn_cli_process } from '#lib/cli.ts';
import { create_mock_task_context } from './test_helpers.ts';

vi.mock('#lib/cli.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/cli.ts')>()),
	find_cli: vi.fn(),
	spawn_cli: vi.fn(async () => ({ ok: true })),
	spawn_cli_process: vi.fn(async () => undefined)
}));

vi.mock('#lib/sveltekit_helpers.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/sveltekit_helpers.ts')>()),
	sveltekit_sync_if_available: vi.fn(async () => undefined)
}));

const original_argv = process.argv;

afterEach(() => {
	process.argv = original_argv;
	vi.clearAllMocks();
});

const ARGS = {
	svelte_check_cli: 'svelte-check',
	typescript_cli: 'tsc',
	path_replacement: '.',
	cwd: process.cwd()
};

// finds only the named CLI
const find_only = (found: string): void => {
	vi.mocked(find_cli).mockImplementation(async (name) =>
		name === found ? { kind: 'local', name, id: '/' + name } : null
	);
};

const run_tsc = async (forwarded: Array<string>): Promise<Array<string>> => {
	find_only('tsc');
	process.argv = ['node', 'gro', 'typecheck', '--', 'tsc', ...forwarded];
	await task.run(create_mock_task_context(ARGS));
	expect(vi.mocked(spawn_cli)).toHaveBeenCalledTimes(1);
	return vi.mocked(spawn_cli).mock.calls[0]![1]!;
};

describe('typecheck task forwarded args', () => {
	test('svelte-check gets the text verbatim', async () => {
		find_only('svelte-check');
		const forwarded = ['--threshold', 'error', '--tsconfig', './x.json', '--no-cache', '--x='];
		process.argv = ['node', 'gro', 'typecheck', '--', 'svelte-check', ...forwarded];
		await task.run(create_mock_task_context(ARGS));
		expect(vi.mocked(spawn_cli_process).mock.calls[0]![1]).toEqual(forwarded);
	});

	test('tsc gets the text verbatim and `--noEmit` last', async () => {
		expect(await run_tsc(['--target', 'es2024', '--maxNodeModuleJsDepth', '01'])).toEqual([
			'--target',
			'es2024',
			'--maxNodeModuleJsDepth',
			'01',
			'--noEmit'
		]);
	});

	test("a forwarded `--build` or `-b` stays tsc's first arg", async () => {
		for (const build of ['--build', '-b']) {
			vi.clearAllMocks();
			expect(await run_tsc([build, 'tsconfig.json'])).toEqual([build, 'tsconfig.json', '--noEmit']);
		}
	});

	test('a forwarded `--noEmit` in any case drops the default', async () => {
		for (const forwarded of [['--noEmit', 'false'], ['--noemit'], ['--NOEMIT=false']]) {
			vi.clearAllMocks();
			expect(await run_tsc(forwarded)).toEqual(forwarded);
		}
	});
});
