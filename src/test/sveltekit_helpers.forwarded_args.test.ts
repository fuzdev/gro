import { describe, test, expect, vi, afterEach } from 'vitest';
import { Logger } from '@fuzdev/fuz_util/log.ts';

import { run_svelte_package, type SveltePackageOptions } from '#lib/sveltekit_helpers.ts';
import { spawn_cli, type Cli } from '#lib/cli.ts';
import { SVELTE_PACKAGE_DEP_NAME, SVELTEKIT_DEP_NAME } from '#lib/constants.ts';

vi.mock('#lib/cli.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/cli.ts')>()),
	spawn_cli: vi.fn(async () => ({ ok: true }))
}));

const original_argv = process.argv;

afterEach(() => {
	process.argv = original_argv;
	vi.clearAllMocks();
});

const PACKAGE_JSON = {
	name: 'a',
	version: '0',
	devDependencies: { [SVELTEKIT_DEP_NAME]: '2', [SVELTE_PACKAGE_DEP_NAME]: '2' }
};

const run = async (
	options: SveltePackageOptions,
	forwarded: Array<string>
): Promise<Array<string>> => {
	process.argv = ['node', 'gro', 'build', '--', 'svelte-package', ...forwarded];
	const cli: Cli = { kind: 'local', name: 'svelte-package', id: '/svelte-package' };
	await run_svelte_package(PACKAGE_JSON, options, cli, new Logger('test'), 'npm');
	expect(vi.mocked(spawn_cli)).toHaveBeenCalledTimes(1);
	return vi.mocked(spawn_cli).mock.calls[0]![1]!;
};

describe('run_svelte_package forwarded args', () => {
	test('a forwarded flag drops the configured option of its name', async () => {
		expect(await run({ types: true, watch: true }, ['--types'])).toEqual(['--watch', '--types']);
	});

	test('configured options first, the forwarded text verbatim after', async () => {
		expect(
			await run({ input: 'src/lib', output: 'dist', tsconfig: './tsconfig.json' }, [
				'--output=x',
				'--no-cache',
				'-foo'
			])
		).toEqual([
			'--input',
			'src/lib',
			'--tsconfig',
			'./tsconfig.json',
			'--output=x',
			'--no-cache',
			'-foo'
		]);
	});

	test('a configured `false` serializes as `--no-x`', async () => {
		expect(await run({ types: false, watch: false }, [])).toEqual(['--no-types', '--no-watch']);
	});

	test('a configured short alias `false` serializes by its long name', async () => {
		expect(await run({ t: false, w: false, i: 'src/lib' }, [])).toEqual([
			'--no-types',
			'--no-watch',
			'-i',
			'src/lib'
		]);
	});

	test('a forwarded `--no-x` or short flag drops the option of that spelling', async () => {
		expect(await run({ types: true, o: 'dist', watch: true }, ['--no-types', '-ox'])).toEqual([
			'--watch',
			'--no-types',
			'-ox'
		]);
	});
});

describe('run_svelte_package failure', () => {
	test('a failed `svelte-package` throws', async () => {
		vi.mocked(spawn_cli).mockResolvedValueOnce({ ok: false } as never);
		process.argv = ['node', 'gro', 'build'];
		const cli: Cli = { kind: 'local', name: 'svelte-package', id: '/svelte-package' };
		await expect(
			run_svelte_package(PACKAGE_JSON, undefined, cli, new Logger('test'), 'npm')
		).rejects.toThrow('`svelte-package` failed');
	});
});
