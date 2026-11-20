import { describe, test, expect, vi, afterEach } from 'vitest';

import { task } from '#lib/typecheck.task.ts';
import { create_mock_builtin_context } from './test_helpers.ts';
import { get_mock_spawn } from './spawn_mocks.ts';

vi.mock('#lib/spawn.ts', async () => (await import('./spawn_mocks.ts')).create_mock_spawn_module());

vi.mock('#lib/sveltekit_helpers.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/sveltekit_helpers.ts')>()),
	sveltekit_sync: vi.fn(async () => undefined)
}));

afterEach(async () => {
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
	await task.run(
		await create_mock_builtin_context(task, ARGS, {
			name: 'typecheck',
			cli_args: ['typecheck', '--', 'tsc', ...forwarded]
		})
	);
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
		await task.run(
			await create_mock_builtin_context(task, ARGS, {
				name: 'typecheck',
				cli_args: ['typecheck', '--', 'svelte-check', ...forwarded]
			})
		);
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

describe('a chosen checker', () => {
	test('`--typescript_cli` names the program run, and its own section is forwarded', async () => {
		await find_only('tsgo');
		await task.run(
			await create_mock_builtin_context(
				task,
				{ ...ARGS, typescript_cli: 'tsgo' },
				{ name: 'typecheck', cli_args: ['typecheck', '--', 'tsgo', '--x', '--', 'tsc', '--y'] }
			)
		);
		const { finds, calls } = await get_mock_spawn();
		expect(finds).toContain('node_modules/.bin/tsgo');
		expect(calls).toHaveLength(1);
		expect(calls[0]!.name).toBe('node_modules/.bin/tsgo');
		expect(calls[0]!.args).toEqual(['--x', '--noEmit']);
		expect(calls[0]!.program_options).toEqual({ pass_env: 'all' });
	});

	test('`--svelte_check_cli` names the program run, and its own section is forwarded', async () => {
		await find_only('svelte-check-next');
		await task.run(
			await create_mock_builtin_context(
				task,
				{ ...ARGS, svelte_check_cli: 'svelte-check-next' },
				{
					name: 'typecheck',
					cli_args: ['typecheck', '--', 'svelte-check-next', '--x', '--', 'svelte-check', '--y']
				}
			)
		);
		const { calls } = await get_mock_spawn();
		expect(calls).toHaveLength(1);
		expect(calls[0]!.name).toBe('node_modules/.bin/svelte-check-next');
		expect(calls[0]!.args).toEqual(['--x']);
		expect(calls[0]!.program_options).toEqual({ pass_env: 'all', env: { FORCE_COLOR: '1' } });
	});
});
