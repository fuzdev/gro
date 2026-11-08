import { describe, test, expect, afterEach } from 'vitest';
import { Logger } from '@fuzdev/fuz_util/log.ts';

import { run_svelte_package, type SveltePackageOptions } from '#lib/sveltekit_helpers.ts';
import { LIB_PATH, SVELTE_PACKAGE_DEP_NAME, SVELTEKIT_DEP_NAME } from '#lib/constants.ts';
import { create_mock_fs_dir, create_mock_project_fs_paths, mock_tree } from './fs_mocks.ts';
import { create_mock_spawn } from './spawn_mocks.ts';

const original_argv = process.argv;

const spawn = create_mock_spawn();

afterEach(() => {
	process.argv = original_argv;
	spawn.reset();
});

const PACKAGE_JSON = {
	name: 'a',
	version: '0',
	devDependencies: { [SVELTEKIT_DEP_NAME]: '2', [SVELTE_PACKAGE_DEP_NAME]: '2' }
};

// a library project, whose lib directory `run_svelte_package` checks for
const to_fs_paths = () => {
	const project = create_mock_fs_dir('/project');
	mock_tree(project, { [LIB_PATH + '/']: '' });
	return create_mock_project_fs_paths(project);
};

const run = async (
	options: SveltePackageOptions,
	forwarded: Array<string>
): Promise<Array<string>> => {
	process.argv = ['node', 'gro', 'build', '--', 'svelte-package', ...forwarded];
	await run_svelte_package(to_fs_paths(), spawn, {
		package_json: PACKAGE_JSON,
		svelte_package_options: options,
		svelte_package_cli: 'svelte-package',
		log: new Logger('test'),
		pm_cli: 'npm'
	});
	expect(spawn.calls).toHaveLength(1);
	expect(spawn.calls[0]!.program_options).toEqual({ pass_env: 'all' });
	return [...spawn.calls[0]!.args];
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
		spawn.result = { kind: 'exited', code: 1 };
		process.argv = ['node', 'gro', 'build'];
		await expect(
			run_svelte_package(to_fs_paths(), spawn, {
				package_json: PACKAGE_JSON,
				svelte_package_cli: 'svelte-package',
				log: new Logger('test'),
				pm_cli: 'npm'
			})
		).rejects.toThrow('`svelte-package` failed: code 1');
	});

	test('a CLI only on `PATH` is not found, since it must be local', async () => {
		spawn.available = (name) => name === 'svelte-package';
		process.argv = ['node', 'gro', 'build'];
		await expect(
			run_svelte_package(to_fs_paths(), spawn, {
				package_json: PACKAGE_JSON,
				svelte_package_cli: 'svelte-package',
				log: new Logger('test'),
				pm_cli: 'npm'
			})
		).rejects.toThrow('Failed to find SvelteKit packaging CLI');
		expect(spawn.finds).toEqual(['node_modules/.bin/svelte-package']);
	});
});
