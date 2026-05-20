import {describe, test, expect} from 'vitest';

import {sanitize_install_env} from '../lib/sync.task.ts';

describe('sanitize_install_env', () => {
	test('preserves unrelated vars, including near-miss names', () => {
		const env: NodeJS.ProcessEnv = {
			PATH: '/usr/bin',
			HOME: '/home/user',
			NODE_PATH: '/usr/lib/node_modules',
			NODE_OPTIONS: '--max-old-space-size=4096',
			npm_config_registry: 'https://registry.npmjs.org/',
			CUSTOM: 'value',
		};
		expect(sanitize_install_env(env)).toEqual(env);
	});

	test('strips prune-triggering vars', () => {
		const result = sanitize_install_env({
			PATH: '/usr/bin',
			NODE_ENV: 'production',
			npm_config_production: 'true',
			npm_config_omit: 'dev',
		});
		expect(result).toEqual({PATH: '/usr/bin'});
	});

	test('returns a fresh object', () => {
		const input: NodeJS.ProcessEnv = {PATH: '/usr/bin'};
		expect(sanitize_install_env(input)).not.toBe(input);
	});

	test('does not mutate its input', () => {
		const input: NodeJS.ProcessEnv = {
			PATH: '/usr/bin',
			NODE_ENV: 'production',
			npm_config_omit: 'dev',
		};
		const snapshot = {...input};
		sanitize_install_env(input);
		expect(input).toEqual(snapshot);
	});

	test('handles an empty env', () => {
		expect(sanitize_install_env({})).toEqual({});
	});
});
