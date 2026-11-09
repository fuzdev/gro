import { describe, test, expect, assert, vi, afterEach } from 'vitest';
import { args_parse } from '@fuzdev/fuz_util/args.ts';

import { Args, task } from '#lib/upgrade.task.ts';
import { TaskError } from '#lib/task.ts';
import { install_with_cache_healing_or_throw } from '#lib/npm_install_helpers.ts';
import { get_mock_spawn } from './spawn_mocks.ts';
import { create_mock_task_context } from './test_helpers.ts';
import { set_mock_process_args } from './process_mocks.ts';

vi.mock('#lib/process.ts', async () =>
	(await import('./process_mocks.ts')).create_mock_process_module()
);

vi.mock('#lib/npm_install_helpers.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/npm_install_helpers.ts')>()),
	install_with_cache_healing_or_throw: vi.fn(async () => undefined)
}));

vi.mock('#lib/spawn.ts', async () => (await import('./spawn_mocks.ts')).create_mock_spawn_module());

vi.mock('#lib/package_json.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/package_json.ts')>()),
	package_json_load: vi.fn(async () => ({
		name: 'a',
		version: '0',
		dependencies: { b: '^1.0.0' }
	}))
}));

afterEach(async () => {
	set_mock_process_args([]);
	vi.clearAllMocks();
	(await get_mock_spawn()).reset();
});

describe('upgrade task forwarded args', () => {
	test('the package manager gets the text verbatim after the install args', async () => {
		const forwarded = ['--no-audit', '--fetch-retries', '01', '--x='];
		set_mock_process_args(['upgrade', '--', 'npm', ...forwarded]);
		const parsed = args_parse({ 'no-pull': true, pull: false, force: true }, Args);
		expect(parsed.ok).toBe(true);
		await task.run(create_mock_task_context(parsed.ok ? parsed.value : {}));
		expect(vi.mocked(install_with_cache_healing_or_throw)).toHaveBeenCalledTimes(1);
		const [, pm_cli, options] = vi.mocked(install_with_cache_healing_or_throw).mock.calls[0]!;
		expect(pm_cli).toBe('npm');
		expect(options?.install_args).toEqual(['b@latest', '--force', ...forwarded]);
	});

	test('`gro sync` re-invokes the local Gro with the whole environment', async () => {
		set_mock_process_args(['upgrade']);
		const parsed = args_parse({ 'no-pull': true, pull: false, force: true }, Args);
		expect(parsed.ok).toBe(true);
		await task.run(create_mock_task_context(parsed.ok ? parsed.value : {}));
		const { calls } = await get_mock_spawn();
		expect(calls.map((c) => [c.method, c.name, c.args, c.program_options])).toEqual([
			['run', 'node_modules/.bin/gro', ['sync'], { pass_env: 'all' }]
		]);
	});

	test('a failed `gro sync` is a TaskError', async () => {
		set_mock_process_args(['upgrade']);
		const mock = await get_mock_spawn();
		mock.result = { kind: 'exited', code: 3 };
		const parsed = args_parse({ 'no-pull': true, pull: false }, Args);
		expect(parsed.ok).toBe(true);
		let error: unknown;
		try {
			await task.run(create_mock_task_context(parsed.ok ? parsed.value : {}));
		} catch (e) {
			error = e;
		}
		assert.instanceOf(error, TaskError);
		assert.include(error.message, '`gro sync` failed: code 3');
	});

	test('--dry runs the package manager with --dry-run, and neither pulls nor syncs', async () => {
		set_mock_process_args(['upgrade']);
		const parsed = args_parse({ dry: true }, Args);
		expect(parsed.ok).toBe(true);
		await task.run(create_mock_task_context(parsed.ok ? parsed.value : {}));
		const [, , options] = vi.mocked(install_with_cache_healing_or_throw).mock.calls[0]!;
		expect(options?.install_args).toEqual(['b@latest', '--dry-run']);
		expect((await get_mock_spawn()).calls).toEqual([]);
	});
});
