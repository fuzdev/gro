import { describe, test, expect, vi, beforeEach } from 'vitest';

import { gro_plugin_deno_server } from '#lib/gro_plugin_deno_server.ts';
import type { PluginContext } from '#lib/plugin.ts';
import { TaskError } from '#lib/task.ts';

import { create_mock_logger } from './test_helpers.ts';
import { create_mock_child, get_mock_spawn } from './spawn_mocks.ts';

vi.mock('#lib/spawn.ts', async () => (await import('./spawn_mocks.ts')).create_mock_spawn_module());

const to_ctx = (overrides: Partial<PluginContext> = {}): PluginContext =>
	({ dev: true, watch: true, log: create_mock_logger(), ...overrides }) as PluginContext;

describe('gro_plugin_deno_server', () => {
	beforeEach(async () => {
		const mock = await get_mock_spawn();
		mock.reset();
		mock.available = (name) => name === 'deno';
	});

	test('pins `deno` at setup with the whole environment and its explicit values', async () => {
		const mock = await get_mock_spawn();
		const plugin = gro_plugin_deno_server({
			entry: 'server.ts',
			port: 4242,
			host: '127.0.0.1',
			env: { EXTRA: '1' }
		});
		await plugin.setup!(to_ctx());
		expect(mock.calls).toHaveLength(1);
		const [call] = mock.calls;
		expect(call!.method).toBe('start');
		expect(call!.name).toBe('deno');
		expect(call!.program_options).toEqual({
			pass_env: 'all',
			omit_env: [],
			env: { PORT: '4242', HOST: '127.0.0.1', NODE_ENV: 'development', EXTRA: '1' }
		});
		expect(call!.args).toEqual([
			'run',
			'--allow-net',
			'--allow-read',
			'--allow-env',
			'--env=.env.development',
			'--watch',
			'server.ts'
		]);
	});

	test('an undefined `env` value unsets the name rather than refusing it at setup', async () => {
		const mock = await get_mock_spawn();
		const plugin = gro_plugin_deno_server({ env: { KEPT: '1', GRO_NOPE: undefined } });
		await plugin.setup!(to_ctx());
		expect(mock.calls[0]!.program_options!.env).toStrictEqual({
			PORT: '4041',
			HOST: 'localhost',
			NODE_ENV: 'development',
			KEPT: '1'
		});
		expect(mock.calls[0]!.program_options!.omit_env).toStrictEqual(['GRO_NOPE']);
	});

	test('watch false drops `--watch`, env_file null drops `--env`', async () => {
		const mock = await get_mock_spawn();
		const plugin = gro_plugin_deno_server({ entry: 's.ts', env_file: null, watch: false });
		await plugin.setup!(to_ctx());
		expect(mock.calls[0]!.args).toEqual([
			'run',
			'--allow-net',
			'--allow-read',
			'--allow-env',
			's.ts'
		]);
	});

	test('a non-dev run starts nothing', async () => {
		const mock = await get_mock_spawn();
		await gro_plugin_deno_server().setup!(to_ctx({ dev: false }));
		expect(mock.finds).toEqual([]);
		expect(mock.calls).toEqual([]);
	});

	test('teardown stops the server, then does nothing more', async () => {
		const mock = await get_mock_spawn();
		const dispose = vi.fn(async () => undefined);
		mock.create_child = () => create_mock_child({ [Symbol.asyncDispose]: dispose });
		const plugin = gro_plugin_deno_server();
		await plugin.setup!(to_ctx());
		await plugin.teardown!(to_ctx());
		expect(dispose).toHaveBeenCalledTimes(1);
		await plugin.teardown!(to_ctx());
		expect(dispose).toHaveBeenCalledTimes(1);
	});

	test('a missing `deno` fails with the host module message, not a silent start', async () => {
		const mock = await get_mock_spawn();
		mock.available = () => false;
		const error = await Promise.resolve(gro_plugin_deno_server().setup!(to_ctx())).catch(
			(e: unknown) => e
		);
		expect(error).toBeInstanceOf(TaskError);
		expect((error as TaskError).message).toContain('not_found');
	});
});
