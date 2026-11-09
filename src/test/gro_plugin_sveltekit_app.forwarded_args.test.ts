import { describe, test, expect, vi, afterEach } from 'vitest';

import { gro_plugin_sveltekit_app } from '#lib/gro_plugin_sveltekit_app.ts';
import type { PluginContext } from '#lib/plugin.ts';
import { create_mock_task_context } from './test_helpers.ts';
import { get_mock_spawn } from './spawn_mocks.ts';
import { set_mock_process_args } from './process_mocks.ts';

vi.mock('#lib/process.ts', async () =>
	(await import('./process_mocks.ts')).create_mock_process_module()
);

vi.mock('#lib/spawn.ts', async () => (await import('./spawn_mocks.ts')).create_mock_spawn_module());

afterEach(async () => {
	set_mock_process_args([]);
	(await get_mock_spawn()).reset();
});

const FORWARDED = ['--port', '0123', '--no-open', '--mode', '-foo', '--x='];

const setup = async (dev: boolean): Promise<void> => {
	set_mock_process_args([dev ? 'dev' : 'build', '--', 'vite', ...FORWARDED]);
	const ctx = { ...create_mock_task_context(), dev, watch: true } as unknown as PluginContext;
	await gro_plugin_sveltekit_app().setup!(ctx);
};

describe('gro_plugin_sveltekit_app forwarded args', () => {
	test('`vite dev` gets the text verbatim', async () => {
		await setup(true);
		const { calls } = await get_mock_spawn();
		expect(calls[0]!.method).toBe('start');
		expect(calls[0]!.args).toEqual(['dev', ...FORWARDED]);
		expect(calls[0]!.program_options).toEqual({ pass_env: 'all' });
	});

	test('`vite build` gets the text verbatim', async () => {
		await setup(false);
		const { calls } = await get_mock_spawn();
		expect(calls[0]!.method).toBe('run');
		expect(calls[0]!.args).toEqual(['build', ...FORWARDED]);
		expect(calls[0]!.program_options).toEqual({ pass_env: 'all' });
	});
});
