import { describe, test, expect, vi, afterEach } from 'vitest';

import { gro_plugin_sveltekit_app } from '#lib/gro_plugin_sveltekit_app.ts';
import { spawn_cli, spawn_cli_process } from '#lib/cli.ts';
import type { PluginContext } from '#lib/plugin.ts';
import { create_mock_task_context } from './test_helpers.ts';

vi.mock('#lib/cli.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/cli.ts')>()),
	find_cli: vi.fn(async (name: string) => ({ kind: 'local', name, id: '/' + name })),
	spawn_cli: vi.fn(async () => ({ ok: true })),
	spawn_cli_process: vi.fn(async () => undefined)
}));

const original_argv = process.argv;

afterEach(() => {
	process.argv = original_argv;
	vi.clearAllMocks();
});

const FORWARDED = ['--port', '0123', '--no-open', '--mode', '-foo', '--x='];

const setup = async (dev: boolean): Promise<void> => {
	process.argv = ['node', 'gro', dev ? 'dev' : 'build', '--', 'vite', ...FORWARDED];
	const ctx = { ...create_mock_task_context(), dev, watch: true } as unknown as PluginContext;
	await gro_plugin_sveltekit_app().setup!(ctx);
};

describe('gro_plugin_sveltekit_app forwarded args', () => {
	test('`vite dev` gets the text verbatim', async () => {
		await setup(true);
		expect(vi.mocked(spawn_cli_process).mock.calls[0]![1]).toEqual(['dev', ...FORWARDED]);
	});

	test('`vite build` gets the text verbatim', async () => {
		await setup(false);
		expect(vi.mocked(spawn_cli).mock.calls[0]![1]).toEqual(['build', ...FORWARDED]);
	});
});
