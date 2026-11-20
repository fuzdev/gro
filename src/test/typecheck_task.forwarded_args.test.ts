import { describe, test, expect, vi, afterEach } from 'vitest';
import { args_parse } from '@fuzdev/fuz_util/args.ts';

import { Args, task } from '#lib/typecheck.task.ts';
import { create_mock_builtin_context } from './test_helpers.ts';
import { get_mock_spawn } from './spawn_mocks.ts';

vi.mock('#lib/spawn.ts', async () => (await import('./spawn_mocks.ts')).create_mock_spawn_module());

vi.mock('#lib/sveltekit_helpers.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/sveltekit_helpers.ts')>()),
	sveltekit_sync: vi.fn(async () => undefined)
}));

vi.mock('#lib/package_json.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/package_json.ts')>()),
	project_has_dependency: vi.fn()
}));

// the project's `src/` has no Svelte files, so an undeclared svelte-check leaves tsc
vi.mock('#lib/svelte_helpers.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/svelte_helpers.ts')>()),
	find_svelte_files: vi.fn(() => Promise.resolve([]))
}));

afterEach(async () => {
	vi.clearAllMocks();
	(await get_mock_spawn()).reset();
});

const ARGS = {
	path_replacement: '.',
	cwd: process.cwd()
};

// declares and finds only the named CLI, `typescript` for tsc
const find_only = async (found: string): Promise<void> => {
	(await get_mock_spawn()).available = (name) => name === 'node_modules/.bin/' + found;
	const declared = found === 'tsc' ? 'typescript' : found;
	vi.mocked(await import('#lib/package_json.ts')).project_has_dependency.mockImplementation(
		(_fs_paths, dep_name) => Promise.resolve(dep_name === declared)
	);
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

describe('the checkers', () => {
	test('`--typescript_cli` and `--svelte_check_cli` are refused', () => {
		expect(args_parse({ typescript_cli: 'tsgo' }, Args).ok).toBe(false);
		expect(args_parse({ svelte_check_cli: 'svelte-check-next' }, Args).ok).toBe(false);
	});

	test("tsc gets its own section, not another tool's", async () => {
		await find_only('tsc');
		await task.run(
			await create_mock_builtin_context(task, ARGS, {
				name: 'typecheck',
				cli_args: ['typecheck', '--', 'tsgo', '--x', '--', 'tsc', '--y']
			})
		);
		const { calls } = await get_mock_spawn();
		expect(calls).toHaveLength(1);
		expect(calls[0]!.name).toBe('node_modules/.bin/tsc');
		expect(calls[0]!.args).toEqual(['--y', '--noEmit']);
		expect(calls[0]!.program_options).toEqual({ pass_env: 'all' });
	});

	test("svelte-check gets its own section, not another tool's", async () => {
		await find_only('svelte-check');
		await task.run(
			await create_mock_builtin_context(task, ARGS, {
				name: 'typecheck',
				cli_args: ['typecheck', '--', 'svelte-check-next', '--x', '--', 'svelte-check', '--y']
			})
		);
		const { calls } = await get_mock_spawn();
		expect(calls).toHaveLength(1);
		expect(calls[0]!.name).toBe('node_modules/.bin/svelte-check');
		expect(calls[0]!.args).toEqual(['--y']);
		expect(calls[0]!.program_options).toEqual({ pass_env: 'all', env: { FORCE_COLOR: '1' } });
	});
});

test('a forwarded help or version flag is refused before either checker runs', async () => {
	for (const [found, flag] of [
		['svelte-check', '-h'],
		['svelte-check', '--version'],
		['tsc', '--help'],
		['tsc', '-v']
	] as const) {
		await find_only(found);
		await expect(
			task.run(
				await create_mock_builtin_context(task, ARGS, {
					name: 'typecheck',
					cli_args: ['typecheck', '--', found, flag]
				})
			)
		).rejects.toThrow(`\`${flag}\` forwarded to ${found} asks it for its `);
	}
	expect((await get_mock_spawn()).calls).toHaveLength(0);
});
