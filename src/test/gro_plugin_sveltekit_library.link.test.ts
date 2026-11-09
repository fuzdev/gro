import { describe, test, expect, vi, beforeEach } from 'vitest';

import { gro_plugin_sveltekit_library } from '#lib/gro_plugin_sveltekit_library.ts';
import type { PluginContext } from '#lib/plugin.ts';
import { TaskError } from '#lib/task.ts';

import { create_mock_task_context } from './test_helpers.ts';
import { get_mock_spawn } from './spawn_mocks.ts';

vi.mock('#lib/spawn.ts', async () => (await import('./spawn_mocks.ts')).create_mock_spawn_module());

vi.mock('#lib/package_json.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/package_json.ts')>()),
	package_json_load: vi.fn(async () => ({ name: 'a', version: '0', bin: { a: 'dist/a.js' } }))
}));

const to_ctx = async (): Promise<PluginContext> =>
	create_mock_task_context({}) as unknown as PluginContext;

describe('gro_plugin_sveltekit_library adapt', () => {
	beforeEach(async () => {
		const mock = await get_mock_spawn();
		mock.reset();
		mock.available = () => true;
	});

	test('chmods each bin with the base environment, then links with the whole one', async () => {
		const mock = await get_mock_spawn();
		await gro_plugin_sveltekit_library().adapt!(await to_ctx());
		expect(mock.calls.map((c) => [c.name, [...c.args], c.program_options])).toEqual([
			['chmod', ['+x', 'dist/a.js'], undefined],
			['npm', ['link', '-f'], { pass_env: 'all' }]
		]);
	});

	test('a failed chmod is logged and linking still runs', async () => {
		const mock = await get_mock_spawn();
		mock.result = (call) =>
			call.name === 'chmod' ? { kind: 'exited', code: 1 } : { kind: 'exited', code: 0 };
		const ctx = await to_ctx();
		await gro_plugin_sveltekit_library().adapt!(ctx);
		expect(mock.calls.map((c) => c.name)).toEqual(['chmod', 'npm']);
		expect(ctx.log.error).toHaveBeenCalledWith('chmod on bin path dist/a.js failed: code 1');
	});

	test('a failed link throws with the result', async () => {
		const mock = await get_mock_spawn();
		mock.result = (call) =>
			call.name === 'npm' ? { kind: 'exited', code: 7 } : { kind: 'exited', code: 0 };
		const error = await Promise.resolve(
			gro_plugin_sveltekit_library().adapt!(await to_ctx())
		).catch((e: unknown) => e);
		expect(error).toBeInstanceOf(TaskError);
		expect((error as TaskError).message).toBe('Failed to link. code 7');
	});

	test('a missing package manager fails the link with the host module message', async () => {
		const mock = await get_mock_spawn();
		mock.available = (name) => name === 'chmod';
		const error = await Promise.resolve(
			gro_plugin_sveltekit_library().adapt!(await to_ctx())
		).catch((e: unknown) => e);
		expect(error).toBeInstanceOf(TaskError);
		expect((error as TaskError).message).toContain('not_found');
	});
});
