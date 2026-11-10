import { describe, test, expect, vi, beforeEach, onTestFinished, assert } from 'vitest';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { gro_plugin_server } from '#lib/gro_plugin_server.ts';
import type { PluginContext } from '#lib/plugin.ts';
import type { ModuleGraph } from '#lib/module_graph.ts';
import { TaskError } from '#lib/task.ts';
import { paths } from '#lib/paths.ts';

import { create_mock_logger } from './test_helpers.ts';
import { create_mock_child, get_mock_spawn } from './spawn_mocks.ts';

vi.mock('#lib/spawn.ts', async () => (await import('./spawn_mocks.ts')).create_mock_spawn_module());
vi.mock('#lib/gro_helpers.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/gro_helpers.ts')>()),
	resolve_gro_module_path: vi.fn(() => '/gro/loader.js')
}));

// a file that exists under `src/`, standing in for a server entry
const ENTRY = 'src/lib/gro_plugin_server.ts';

const create_graph = (): ModuleGraph & { watches: number } => {
	const graph = {
		watches: 0,
		modules: () => Promise.reject(Error('unused')),
		dependents: async () => new Set<string>(),
		watch: async () => {
			graph.watches++;
			return async () => undefined;
		},
		close: async () => undefined
	};
	return graph;
};

const to_ctx = (overrides: Partial<PluginContext> = {}): PluginContext =>
	({
		dev: true,
		watch: true,
		log: create_mock_logger(),
		config: { watch: { dir: paths.source.slice(0, -1), filter: null } },
		graph: create_graph(),
		...overrides
	}) as unknown as PluginContext;

const create_env_file = async (contents: string): Promise<string> => {
	const dir = await mkdtemp(join(tmpdir(), 'gro_plugin_server_'));
	onTestFinished(() => rm(dir, { recursive: true, force: true }));
	const path = join(dir, '.env.test');
	await writeFile(path, contents);
	return path;
};

describe('gro_plugin_server', () => {
	beforeEach(async () => {
		const mock = await get_mock_spawn();
		mock.reset();
		mock.available = (name) => name === 'node';
	});

	test('pins `node` with the whole environment and runs the entry with the loader', async () => {
		const mock = await get_mock_spawn();
		const plugin = gro_plugin_server({ entry: ENTRY, env_file: null, env: { PORT: '4242' } });
		await plugin.setup!(to_ctx());
		expect(mock.calls).toHaveLength(1);
		const [call] = mock.calls;
		expect(call!.method).toBe('start');
		expect(call!.name).toBe('node');
		expect(call!.program_options).toEqual({
			pass_env: 'all',
			env: { NODE_ENV: 'development', PORT: '4242' }
		});
		expect(call!.args.at(-1)).toBe(ENTRY);
		expect(call!.args.join(' ')).toContain('register("/gro/loader.js"');
		expect(call!.args).toContain('development');
		await plugin.teardown!(to_ctx());
	});

	test("the env file's vars go beneath the host's and `env`, which wins over both", async () => {
		const mock = await get_mock_spawn();
		const env_file = await create_env_file(
			'GRO_TEST_FILE_ONLY=file\nHOME=file\nGRO_TEST_BOTH=file\nNODE_ENV=production\n'
		);
		const plugin = gro_plugin_server({ entry: ENTRY, env_file, env: { GRO_TEST_BOTH: 'env' } });
		await plugin.setup!(to_ctx());
		// `HOME` is set on the host, so the file's value is dropped and the host's passes
		expect(mock.calls[0]!.program_options!.env).toStrictEqual({
			GRO_TEST_FILE_ONLY: 'file',
			GRO_TEST_BOTH: 'env',
			NODE_ENV: 'development'
		});
		await plugin.teardown!(to_ctx());
	});

	test('`env` sets `NODE_ENV`, which the loader args follow', async () => {
		const mock = await get_mock_spawn();
		const plugin = gro_plugin_server({
			entry: ENTRY,
			env_file: null,
			env: { NODE_ENV: 'production' }
		});
		await plugin.setup!(to_ctx());
		expect(mock.calls[0]!.program_options!.env).toStrictEqual({ NODE_ENV: 'production' });
		expect(mock.calls[0]!.args).not.toContain('development');
		await plugin.teardown!(to_ctx());
	});

	test('a missing env file warns and loads nothing', async () => {
		const mock = await get_mock_spawn();
		const log = create_mock_logger();
		const plugin = gro_plugin_server({ entry: ENTRY, env_file: '/nonexistent/.env.nope' });
		await plugin.setup!(to_ctx({ log }));
		expect(log.warn).toHaveBeenCalledWith(
			'[gro_plugin_server] env file not found, loading none: /nonexistent/.env.nope'
		);
		expect(mock.calls[0]!.program_options!.env).toStrictEqual({ NODE_ENV: 'development' });
		await plugin.teardown!(to_ctx());
	});

	test('watches the graph by default, and not with watch false', async () => {
		const mock = await get_mock_spawn();
		const graph = create_graph();
		const plugin = gro_plugin_server({ entry: ENTRY, env_file: null });
		await plugin.setup!(to_ctx({ graph }));
		expect(graph.watches).toBe(1);
		await plugin.teardown!(to_ctx());

		const off = gro_plugin_server({ entry: ENTRY, env_file: null, watch: false });
		await off.setup!(to_ctx({ graph }));
		await off.teardown!(to_ctx());
		expect(graph.watches).toBe(1);
		expect(mock.calls).toHaveLength(2);
	});

	test('`gro dev --no-watch` starts nothing, since it tears down right after setup', async () => {
		const mock = await get_mock_spawn();
		const graph = create_graph();
		const log = create_mock_logger();
		await gro_plugin_server({ entry: ENTRY }).setup!(to_ctx({ graph, log, watch: false }));
		expect(mock.calls).toEqual([]);
		expect(graph.watches).toBe(0);
		expect(log.info).toHaveBeenCalledWith(
			`[gro_plugin_server] not starting ${ENTRY} without watch`
		);
	});

	test('an entry outside watch.dir warns that its changes restart nothing', async () => {
		const log = create_mock_logger();
		const plugin = gro_plugin_server({ entry: 'package.json', env_file: null });
		await plugin.setup!(to_ctx({ log }));
		expect(log.warn).toHaveBeenCalledTimes(1);
		expect((log.warn as any).mock.calls[0][0]).toContain('package.json is outside watch.dir');
		await plugin.teardown!(to_ctx());
	});

	test('an entry the watch filter rejects warns the same way', async () => {
		const log = create_mock_logger();
		const plugin = gro_plugin_server({ entry: ENTRY, env_file: null });
		const config = { watch: { dir: paths.source.slice(0, -1), filter: () => false } };
		await plugin.setup!(to_ctx({ log, config } as any));
		expect(log.warn).toHaveBeenCalledTimes(1);
		expect((log.warn as any).mock.calls[0][0]).toContain(
			`${ENTRY} is outside watch.dir ${paths.source.slice(0, -1)} or rejected by watch.filter`
		);
		await plugin.teardown!(to_ctx());
	});

	test('a non-dev run starts nothing', async () => {
		const mock = await get_mock_spawn();
		await gro_plugin_server({ entry: ENTRY }).setup!(to_ctx({ dev: false }));
		expect(mock.finds).toEqual([]);
		expect(mock.calls).toEqual([]);
	});

	test('a missing entry fails setup', async () => {
		const mock = await get_mock_spawn();
		const error = await Promise.resolve(
			gro_plugin_server({ entry: 'src/nope.ts' }).setup!(to_ctx())
		).catch((e: unknown) => e);
		assert(error instanceof TaskError);
		expect(error.message).toBe('[gro_plugin_server] cannot find the server entry: src/nope.ts');
		expect(mock.calls).toEqual([]);
	});

	test('a missing `node` fails with the host module message', async () => {
		const mock = await get_mock_spawn();
		mock.available = () => false;
		const graph = create_graph();
		const error = await Promise.resolve(
			gro_plugin_server({ entry: ENTRY, env_file: null }).setup!(to_ctx({ graph }))
		).catch((e: unknown) => e);
		assert(error instanceof TaskError);
		expect(error.message).toContain('not_found');
	});

	test('teardown stops the server, then does nothing more', async () => {
		const mock = await get_mock_spawn();
		const dispose = vi.fn(async () => undefined);
		mock.create_child = () => create_mock_child({ [Symbol.asyncDispose]: dispose });
		const plugin = gro_plugin_server({ entry: ENTRY, env_file: null });
		await plugin.setup!(to_ctx());
		await plugin.teardown!(to_ctx());
		expect(dispose).toHaveBeenCalledTimes(1);
		await plugin.teardown!(to_ctx());
		expect(dispose).toHaveBeenCalledTimes(1);
	});
});
