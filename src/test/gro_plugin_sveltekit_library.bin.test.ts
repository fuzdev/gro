import { describe, test, expect, vi, beforeEach } from 'vitest';

import { gro_plugin_sveltekit_library } from '#lib/gro_plugin_sveltekit_library.ts';
import type { PluginContext } from '#lib/plugin.ts';

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

	test('chmods each bin with the base environment and never links', async () => {
		const mock = await get_mock_spawn();
		await gro_plugin_sveltekit_library().adapt!(await to_ctx());
		expect(mock.calls.map((c) => [c.name, [...c.args], c.program_options])).toEqual([
			['chmod', ['+x', 'dist/a.js'], undefined]
		]);
	});

	test('a failed chmod is logged, not thrown', async () => {
		const mock = await get_mock_spawn();
		mock.result = { kind: 'exited', code: 1 };
		const ctx = await to_ctx();
		await gro_plugin_sveltekit_library().adapt!(ctx);
		expect(ctx.log.error).toHaveBeenCalledWith('chmod on bin path dist/a.js failed: code 1');
	});

	test('a missing chmod is logged with the host module message', async () => {
		const mock = await get_mock_spawn();
		mock.available = () => false;
		const ctx = await to_ctx();
		await gro_plugin_sveltekit_library().adapt!(ctx);
		expect(vi.mocked(ctx.log.error).mock.calls[0]![0]).toContain('not_found');
	});
});
