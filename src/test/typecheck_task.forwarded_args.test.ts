import { describe, test, expect, vi, afterEach } from 'vitest';

import { task } from '#lib/typecheck.task.ts';
import { create_mock_task_context } from './test_helpers.ts';
import { get_mock_spawn } from './spawn_mocks.ts';
import { set_mock_process_args } from './process_mocks.ts';

vi.mock('#lib/process.ts', async () =>
	(await import('./process_mocks.ts')).create_mock_process_module()
);

vi.mock('#lib/spawn.ts', async () => (await import('./spawn_mocks.ts')).create_mock_spawn_module());

vi.mock('#lib/sveltekit_helpers.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/sveltekit_helpers.ts')>()),
	sveltekit_sync: vi.fn(async () => undefined)
}));

afterEach(async () => {
	set_mock_process_args([]);
	vi.clearAllMocks();
	(await get_mock_spawn()).reset();
});

const ARGS = {
	svelte_check_cli: 'svelte-check',
	typescript_cli: 'tsc',
	path_replacement: '.',
	cwd: process.cwd()
};

// finds only the named CLI
const find_only = async (found: string): Promise<void> => {
	(await get_mock_spawn()).available = (name) => name === 'node_modules/.bin/' + found;
};

const run_tsc = async (forwarded: Array<string>): Promise<Array<string>> => {
	await find_only('tsc');
	set_mock_process_args(['typecheck', '--', 'tsc', ...forwarded]);
	await task.run(create_mock_task_context(ARGS));
	const { calls } = await get_mock_spawn();
	expect(calls).toHaveLength(1);
	expect(calls[0]!.method).toBe('run');
	expect(calls[0]!.program_options).toEqual({ pass_env: 'all' });
	return [...calls[0]!.args];
};

describe('typecheck task forwarded args', () => {
	test('svelte-check gets the text verbatim', async () => {
		await find_only('svelte-check');
		const forwarded = ['--threshold', 'error', '--tsconfig', './x.json', '--no-cache', '--x='];
		set_mock_process_args(['typecheck', '--', 'svelte-check', ...forwarded]);
		await task.run(create_mock_task_context(ARGS));
		const { calls } = await get_mock_spawn();
		expect(calls[0]!.method).toBe('start');
		expect(calls[0]!.args).toEqual(forwarded);
		expect(calls[0]!.options).toEqual({ stdio: ['inherit', 'pipe', 'pipe'] });
		expect(calls[0]!.program_options).toEqual({ pass_env: 'all', env: { FORCE_COLOR: '1' } });
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
			(await get_mock_spawn()).reset();
			expect(await run_tsc([build, 'tsconfig.json'])).toEqual([build, 'tsconfig.json', '--noEmit']);
		}
	});

	test('a forwarded `--noEmit` in any case drops the default', async () => {
		for (const forwarded of [['--noEmit', 'false'], ['--noemit'], ['--NOEMIT=false']]) {
			(await get_mock_spawn()).reset();
			expect(await run_tsc(forwarded)).toEqual(forwarded);
		}
	});
});
