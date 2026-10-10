import { describe, test, expect } from 'vitest';
import { resolve } from 'node:path';
import { spawn } from '@fuzdev/fuz_util/process.ts';

import { resolve_gro_module_path, spawn_with_loader_args } from '#lib/gro_helpers.ts';
import { resolve_env_vars, type EnvVarDeclarations } from '#lib/env.ts';
import { render_sveltekit_shim_app_env_vars } from '#lib/sveltekit_shim_app_env_vars.ts';

import { TEST_TIMEOUT_MD } from './test_helpers.ts';

const FIXTURE_DIR = resolve('src/test/fixtures/sveltekit_app_env_project');
const CIRCULAR_FIXTURE_DIR = resolve('src/test/fixtures/sveltekit_app_env_circular_project');

const spawn_fixture = (dir: string, env?: Record<string, string>) =>
	spawn(
		'node',
		spawn_with_loader_args(resolve_gro_module_path('loader.js'), resolve(dir, 'main.ts'), []),
		{ cwd: dir, env: { ...process.env, ...env } }
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

describe('render_sveltekit_shim_app_env_vars', () => {
	test('renders an empty module without declarations', () => {
		const source = render_sveltekit_shim_app_env_vars({
			visibility: 'public',
			variables: null,
			entry_specifier: null,
			dev: true
		});
		expect(source).toContain('export {};');
	});

	test('exports only the declared vars of the visibility', () => {
		const source = render_sveltekit_shim_app_env_vars({
			visibility: 'public',
			variables: { A: { public: true }, B: {} },
			entry_specifier: '/project/src/env.ts',
			dev: true
		});
		expect(source).toContain('export const A = __gro_env.A;');
		expect(source).toContain('__gro_load_env(true, undefined, undefined, undefined)');
		expect(source).not.toContain('export const B');
	});

	// declared names can't collide with the module's own bindings
	test('prefixes its own bindings', () => {
		const source = render_sveltekit_shim_app_env_vars({
			visibility: 'private',
			variables: { env: {}, variables: {}, load_env: {} },
			entry_specifier: '/project/src/env.ts',
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
				SOME_PRIVATE_VAR: 'private_value'
			});
			expect(result.ok).toBe(true);
		},
		TEST_TIMEOUT_MD
	);

	// without the guard the import waits on itself and the process exits 0 having run nothing
	test(
		'fails loudly when the env declarations import the module they declare',
		async () => {
			const result = await spawn_fixture(CIRCULAR_FIXTURE_DIR, { SOME_PUBLIC_VAR: 'x' });
			expect(result.ok).toBe(false);
		},
		TEST_TIMEOUT_MD
	);
});
