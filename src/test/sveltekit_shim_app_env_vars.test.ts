import { describe, test, expect } from 'vitest';
import { spawnSync } from 'node:child_process';
import { resolve } from 'node:path';

import { spawn_with_loader_args } from '#lib/gro_helpers.ts';
import { resolve_env_vars } from '#lib/env_files.ts';
import type { EnvVarDeclarations } from '#lib/env_declarations.ts';
import { render_sveltekit_shim_app_env_vars } from '#lib/sveltekit_shim_app_env_vars.ts';

import { LOADER_SOURCE_PATH, TEST_TIMEOUT_MD } from './test_helpers.ts';

const FIXTURE_DIR = resolve('src/test/fixtures/sveltekit_app_env_project');
const CIRCULAR_FIXTURE_DIR = resolve('src/test/fixtures/sveltekit_app_env_circular_project');

const spawn_fixture = async (dir: string, env?: Record<string, string>) =>
	spawnSync(
		process.execPath,
		spawn_with_loader_args(LOADER_SOURCE_PATH, resolve(dir, 'main.ts'), [], undefined),
		{ cwd: dir, env: { ...process.env, ...env }, stdio: 'inherit' }
	);

describe('resolve_env_vars', () => {
	const variables: EnvVarDeclarations = {
		A: { public: true },
		B: {},
		C: {
			schema: {
				'~standard': { version: 1, vendor: 'test', validate: (v) => ({ value: v ?? 'c' }) }
			}
		}
	};

	test('resolves only the vars of the given visibility', () => {
		expect(resolve_env_vars(variables, 'public', { A: 'a', B: 'b' })).toEqual({ A: 'a' });
		expect(resolve_env_vars(variables, 'private', { A: 'a', B: 'b' })).toEqual({ B: 'b', C: 'c' });
	});

	test('throws for a missing var without a schema', () => {
		expect(() => resolve_env_vars(variables, 'private', {})).toThrow(/B: value is missing/);
	});

	test('runs function validators that skipped `defineEnvVars`', () => {
		const port: EnvVarDeclarations = {
			PORT: {
				schema: (v) => {
					if (v === undefined) return 3000;
					if (!/^\d+$/.test(v)) throw Error('expected an integer');
					return Number(v);
				}
			}
		};
		expect(resolve_env_vars(port, 'private', {})).toEqual({ PORT: 3000 });
		expect(resolve_env_vars(port, 'private', { PORT: '8080' })).toEqual({ PORT: 8080 });
		expect(() => resolve_env_vars(port, 'private', { PORT: 'x' })).toThrow(
			/PORT: expected an integer/
		);
	});

	test('rejects async validators and reports every invalid var at once', () => {
		const vars: EnvVarDeclarations = { A: { schema: async (v) => v }, B: {} };
		expect(() => resolve_env_vars(vars, 'private', {})).toThrow(
			/A: async validators are not supported\n- B: value is missing/
		);
	});
});

const ENV_FILES_MODULE_SPECIFIER = 'file:///gro/env_files.ts';
const FS_MODULE_SPECIFIER = 'file:///gro/fs.ts';
const FS_PATHS_MODULE_SPECIFIER = 'file:///gro/fs_paths.ts';
const ENV_MODULE_SPECIFIER = 'file:///gro/env.ts';
const specifiers = {
	env_files_module_specifier: ENV_FILES_MODULE_SPECIFIER,
	fs_module_specifier: FS_MODULE_SPECIFIER,
	fs_paths_module_specifier: FS_PATHS_MODULE_SPECIFIER,
	env_module_specifier: ENV_MODULE_SPECIFIER
};

describe('render_sveltekit_shim_app_env_vars', () => {
	test('renders an empty module without declarations', () => {
		const source = render_sveltekit_shim_app_env_vars({
			visibility: 'public',
			entry: null,
			...specifiers,
			dev: true
		});
		expect(source).toContain('export {};');
	});

	test('exports only the declared vars of the visibility', () => {
		const source = render_sveltekit_shim_app_env_vars({
			visibility: 'public',
			entry: { id: '/project/src/env.ts', variables: { A: { public: true }, B: {} } },
			...specifiers,
			dev: true
		});
		expect(source).toContain('export const A = __gro_env.A;');
		expect(source).toContain('...{"dev":true},\n\t\tambient_env: __gro_host_env.get_all()');
		expect(source).not.toContain('export const B');
	});

	test('renders the given load_env options', () => {
		const source = render_sveltekit_shim_app_env_vars({
			visibility: 'private',
			entry: { id: '/project/src/env.ts', variables: { A: {} } },
			...specifiers,
			dev: false,
			env_dir: '/project'
		});
		expect(source).toContain(
			'...{"dev":false,"env_dir":"/project"},\n\t\tambient_env: __gro_host_env.get_all()'
		);
		expect(source).toContain(`} from ${JSON.stringify(ENV_FILES_MODULE_SPECIFIER)};`);
	});

	// the rendered module is the root of its env read, its fs and env reach visible in its imports
	test('imports the fs and env host modules and builds the `FsPaths` it reads through', () => {
		const source = render_sveltekit_shim_app_env_vars({
			visibility: 'private',
			entry: { id: '/project/src/env.ts', variables: { A: {} } },
			...specifiers,
			dev: true
		});
		expect(source).toContain(
			`import {fs as __gro_fs} from ${JSON.stringify(FS_MODULE_SPECIFIER)};`
		);
		expect(source).toContain(
			`import {create_fs_paths as __gro_create_fs_paths} from ${JSON.stringify(FS_PATHS_MODULE_SPECIFIER)};`
		);
		expect(source).toContain(
			`import {env as __gro_host_env} from ${JSON.stringify(ENV_MODULE_SPECIFIER)};`
		);
	});

	// declared names can't collide with the module's own bindings
	test('prefixes its own bindings', () => {
		const source = render_sveltekit_shim_app_env_vars({
			visibility: 'private',
			entry: { id: '/project/src/env.ts', variables: { env: {}, variables: {}, load_env: {} } },
			...specifiers,
			dev: true
		});
		expect(source).toContain('export const env = __gro_env.env;');
		expect(source).toContain('export const variables = __gro_env.variables;');
	});
});

describe('sveltekit shim app env', () => {
	test(
		'shims SvelteKit $app/env imports in the loader',
		async () => {
			const result = await spawn_fixture(FIXTURE_DIR, {
				SOME_PUBLIC_VAR: 'public_value',
				SOME_PRIVATE_VAR: 'secret_value'
			});
			expect(result.status).toBe(0);
		},
		TEST_TIMEOUT_MD
	);

	// without the guard the import waits on itself and the process exits 0 having run nothing
	test(
		'fails loudly when the env declarations import the module they declare',
		async () => {
			const result = await spawn_fixture(CIRCULAR_FIXTURE_DIR, { SOME_PUBLIC_VAR: 'x' });
			expect(result.status).not.toBe(0);
		},
		TEST_TIMEOUT_MD
	);
});
