import {describe, test, expect} from 'vitest';

import {
	sanitize_install_env,
	is_etarget_error,
	install_with_cache_healing,
	install_with_cache_healing_or_throw,
	type NpmCommandResult,
	type NpmCommandRunner,
} from '../lib/npm_install_helpers.ts';
import {TaskError} from '../lib/task.ts';

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

describe('is_etarget_error', () => {
	test('detects the ETARGET shapes from message or stderr', () => {
		expect(is_etarget_error('', 'npm error code ETARGET')).toBe(true);
		expect(is_etarget_error('code ETARGET', '')).toBe(true);
		expect(is_etarget_error('', 'notarget')).toBe(true);
		expect(is_etarget_error('', 'No matching version found for @scope/pkg@1.2.3')).toBe(true);
	});

	test('is case-insensitive', () => {
		expect(is_etarget_error('', 'etarget')).toBe(true);
	});

	test('does not match unrelated failures', () => {
		expect(is_etarget_error('code 1', 'npm error EACCES permission denied')).toBe(false);
		expect(is_etarget_error('', '')).toBe(false);
	});
});

/**
 * Builds a fake `NpmCommandRunner` that returns the given results in order and
 * records every call. Missing results default to a successful run.
 */
const make_run = (
	results: Array<{ok: boolean; stderr?: string; detail?: string}>,
): {
	run: NpmCommandRunner;
	calls: Array<{command: string; args: Array<string>; env?: NodeJS.ProcessEnv}>;
} => {
	const calls: Array<{command: string; args: Array<string>; env?: NodeJS.ProcessEnv}> = [];
	let i = 0;
	const run: NpmCommandRunner = async (command, args, options) => {
		calls.push({command, args: [...args], env: options?.env});
		const r = results[i++] ?? {ok: true};
		const result: NpmCommandResult = {
			ok: r.ok,
			stderr: r.stderr ?? '',
			detail: r.detail ?? (r.ok ? 'ok' : 'code 1'),
		};
		return result;
	};
	return {run, calls};
};

describe('install_with_cache_healing', () => {
	test('succeeds on the first attempt without cache cleaning', async () => {
		const {run, calls} = make_run([{ok: true}]);
		const result = await install_with_cache_healing('npm', {run});
		expect(result.ok).toBe(true);
		expect(calls).toHaveLength(1);
		expect(calls[0]!.args).toEqual(['install']);
	});

	test('returns a non-ETARGET failure immediately without healing', async () => {
		const {run, calls} = make_run([{ok: false, stderr: 'npm error EACCES permission denied'}]);
		const result = await install_with_cache_healing('npm', {run});
		expect(result.ok).toBe(false);
		expect(calls).toHaveLength(1);
	});

	test('heals an ETARGET failure: cache clean then retry', async () => {
		const {run, calls} = make_run([
			{ok: false, stderr: 'npm error code ETARGET'},
			{ok: true}, // cache clean
			{ok: true}, // retry install
		]);
		const result = await install_with_cache_healing('npm', {run});
		expect(result.ok).toBe(true);
		expect(calls).toHaveLength(3);
		expect(calls[1]!.args).toEqual(['cache', 'clean', '--force']);
		expect(calls[2]!.args).toEqual(['install']);
	});

	test('returns the retry failure when the heal does not fix it', async () => {
		const {run, calls} = make_run([
			{ok: false, stderr: 'ETARGET'},
			{ok: true}, // cache clean
			{ok: false, stderr: 'ETARGET still', detail: 'code 1'}, // retry install
		]);
		const result = await install_with_cache_healing('npm', {run});
		expect(result.ok).toBe(false);
		expect(calls).toHaveLength(3);
	});

	test('reports a cache-clean failure and does not retry the install', async () => {
		const {run, calls} = make_run([
			{ok: false, stderr: 'ETARGET'},
			{ok: false, detail: 'code 1'}, // cache clean fails
		]);
		const result = await install_with_cache_healing('npm', {run});
		expect(result.ok).toBe(false);
		expect(result.detail).toContain('cache clean');
		expect(calls).toHaveLength(2);
	});

	test('sanitizes the env passed to the runner', async () => {
		const {run, calls} = make_run([{ok: true}]);
		await install_with_cache_healing('npm', {
			run,
			env: {PATH: '/usr/bin', NODE_ENV: 'production'},
		});
		expect(calls[0]!.env).toEqual({PATH: '/usr/bin'});
	});

	test('appends install_args and reuses them on the cache-heal retry', async () => {
		const {run, calls} = make_run([
			{ok: false, stderr: 'ETARGET'},
			{ok: true}, // cache clean
			{ok: true}, // retry install
		]);
		const result = await install_with_cache_healing('npm', {
			run,
			install_args: ['foo@latest', '--force'],
		});
		expect(result.ok).toBe(true);
		expect(calls[0]!.args).toEqual(['install', 'foo@latest', '--force']);
		expect(calls[1]!.args).toEqual(['cache', 'clean', '--force']);
		expect(calls[2]!.args).toEqual(['install', 'foo@latest', '--force']);
	});

	test('only npm self-heals — a non-npm pm_cli surfaces ETARGET without cache cleaning', async () => {
		const {run, calls} = make_run([{ok: false, stderr: 'npm error code ETARGET'}]);
		const result = await install_with_cache_healing('some-other-cli', {run});
		expect(result.ok).toBe(false);
		expect(calls).toHaveLength(1); // no cache clean, no retry
	});
});

describe('install_with_cache_healing_or_throw', () => {
	test('resolves without throwing on success', async () => {
		const {run} = make_run([{ok: true}]);
		await expect(install_with_cache_healing_or_throw('npm', {run})).resolves.toBeUndefined();
	});

	test('throws a TaskError naming the command, context, and detail on failure', async () => {
		const {run} = make_run([{ok: false, detail: 'code 1'}]);
		const promise = install_with_cache_healing_or_throw('npm', {
			run,
			install_args: ['foo@latest'],
			context: 'after version bump',
		});
		await expect(promise).rejects.toBeInstanceOf(TaskError);
		await expect(promise).rejects.toThrowError(
			'Failed `npm install foo@latest` after version bump: code 1',
		);
	});
});
