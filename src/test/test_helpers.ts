import { readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { assert, vi } from 'vitest';
import type { Logger } from '@fuzdev/fuz_util/log.ts';
import type { Timings } from '@fuzdev/fuz_util/timings.ts';
import { json_stringify_deterministic } from '@fuzdev/fuz_util/json.ts';
import { hash_blake3 } from '@fuzdev/fuz_util/hash_blake3.ts';

import type { GroConfig } from '#lib/gro_config.ts';
import { create_paths } from '#lib/paths.ts';
import type { ModuleGraph } from '#lib/module_graph.ts';
import { load_default_svelte_config, type ParsedSvelteConfig } from '#lib/svelte_config.ts';
import type { TaskContext } from '#lib/task.ts';
import type { PluginContext, PluginHost } from '#lib/plugin.ts';
import {
	create_builtin_host,
	type BuiltinHostRoot,
	type BuiltinTask,
	type BuiltinTaskContext,
	type BuiltinTaskGrants
} from '#lib/builtin_host.ts';
import type { Spawn } from '#lib/spawn.ts';
import type { ProcessOutput } from '#lib/stdout.ts';
import type { FsPaths } from '#lib/fs_paths.ts';
import { to_implicit_rest_args, to_raw_rest_args, to_task_args } from '#lib/task_args.ts';
import type { GroPlatform } from '#lib/invoke_task.ts';
import type { GroPackage } from '#lib/package_json.ts';
import { create_node_module_graph } from '#lib/module_graph_node.ts';

/**
 * The paths of the project the tests run in, Gro's own repo, from the working directory.
 */
export const paths = create_paths(process.cwd());

/**
 * A `BuiltinHostRoot` whose handles throw on any use, for a test whose tasks get no `host`,
 * so a task that reaches one fails rather than running on real handles.
 */
export const create_throwing_builtin_host_root = (): BuiltinHostRoot => {
	const create_stub = <T extends object>(label: string): T =>
		new Proxy({} as T, {
			get: (_target, key) => {
				throw Error(`the throwing builtin host root's \`${label}\` was used: ${String(key)}`);
			}
		});
	return {
		env: create_stub('env'),
		spawn: create_stub('spawn'),
		stdout: create_stub('stdout'),
		stderr: create_stub('stderr')
	};
};

/**
 * The running Gro as a test's tasks see it, Gro's own repo, where the tests run.
 */
export const TEST_GRO_PACKAGE: GroPackage = Object.freeze({
	version: JSON.parse(readFileSync(join(paths.root, 'package.json'), 'utf8')).version,
	dir: paths.root
});

/**
 * The platform a test's tasks run with, the Node Gro's but for `TEST_GRO_PACKAGE`.
 */
export const create_test_platform = (): GroPlatform => ({
	create_module_graph: create_node_module_graph,
	load_svelte_config: load_default_svelte_config,
	gro: TEST_GRO_PACKAGE
});

/**
 * What `create_mock_builtin_context` builds a builtin's context from beside its args.
 */
export interface MockBuiltinContextOptions {
	/**
	 * The CLI's args, as `invoke.ts` reads them: their rest from the first `--` is the host's
	 * `raw_rest_args`, and its first section the context's `implicit_rest_args`
	 * when they name `name`, as `invoke_cli` gives the task it invokes.
	 */
	cli_args?: ReadonlyArray<string>;
	/**
	 * The builtin's name, compared with the task `cli_args` names.
	 */
	name?: string;
	config_overrides?: Partial<GroConfig>;
	/**
	 * The root's spawn, by default the spawn host module, which a test may mock.
	 */
	spawn?: Spawn;
	stdout?: ProcessOutput;
	stderr?: ProcessOutput;
	fs_paths?: FsPaths;
}

/**
 * A builtin's context with a real `host` cut by its grants from a root of the env host module,
 * the spawn host module (or `spawn`), and the given streams,
 * so a test sees the programs and environment the grants give.
 */
export const create_mock_builtin_context = async <
	TArgs extends object,
	TGrants extends BuiltinTaskGrants
>(
	task: BuiltinTask<TArgs, TGrants>,
	task_args: Partial<TArgs>,
	options: MockBuiltinContextOptions = {}
): Promise<BuiltinTaskContext<TArgs, TGrants>> => {
	const { cli_args = [], name, config_overrides } = options;
	const spawn = options.spawn ?? (await import('#lib/spawn.ts')).spawn;
	const { env } = await import('#lib/env.ts');
	const { create_fs_paths } = await import('#lib/fs_paths.ts');
	const { fs } = await import('#lib/fs.ts');
	const no_output: ProcessOutput = { write: () => Promise.resolve() } as unknown as ProcessOutput;
	const host = create_builtin_host(
		{ env, spawn, stdout: options.stdout ?? no_output, stderr: options.stderr ?? no_output },
		task.grants,
		{
			fs_paths:
				options.fs_paths ?? create_fs_paths(fs, { cwd: process.cwd(), home: process.env.HOME }),
			import_module: vi.fn()
		}
	);
	const raw_rest_args = to_raw_rest_args(cli_args);
	return {
		...create_mock_task_context<TArgs>(task_args, config_overrides),
		implicit_rest_args: Object.freeze(
			name !== undefined && to_task_args(cli_args).task_name === name
				? to_implicit_rest_args(raw_rest_args)
				: []
		),
		raw_rest_args: Object.freeze(raw_rest_args),
		host
	};
};

/**
 * Asserts that `fn` throws an error `matches` accepts.
 */
export const assert_throws_matching = (
	fn: () => unknown,
	matches: (error: unknown) => boolean
): void => {
	try {
		fn();
	} catch (error) {
		assert.ok(matches(error), `unexpected error: ${String(error)}`);
		return;
	}
	assert.fail('expected a throw');
};

/**
 * Creates a mock logger for testing.
 */
export const create_mock_logger = (): Logger =>
	({
		error: vi.fn(),
		warn: vi.fn(),
		info: vi.fn(),
		debug: vi.fn(),
		raw: vi.fn()
	}) as unknown as Logger;

/**
 * Creates a mock Gro config for testing.
 * Note: build_cache_config in overrides will be hashed during creation.
 */
export const create_mock_config = async (
	overrides: Partial<GroConfig> & {
		build_cache_config?:
			Record<string, unknown> | (() => Record<string, unknown> | Promise<Record<string, unknown>>);
	} = {}
): Promise<GroConfig> => {
	// Extract and hash build_cache_config if provided
	const { build_cache_config, ...rest } = overrides;
	let build_cache_config_hash: string;

	if (!build_cache_config) {
		build_cache_config_hash = hash_blake3('');
	} else {
		// Resolve if it's a function
		const resolved =
			typeof build_cache_config === 'function' ? await build_cache_config() : build_cache_config;

		// Hash the JSON representation with deterministic key ordering
		build_cache_config_hash = hash_blake3(json_stringify_deterministic(resolved));
	}

	return {
		plugins: () => [],
		map_package_json: null,
		search_filters: [],
		watch: { dir: paths.source, filter: null },
		build_cache_config_hash,
		...(rest as Partial<GroConfig>)
	};
};

export const TEST_TIMEOUT_MD = 20_000;

/**
 * Gro's source loader, which tests spawn rather than a built `dist/` that may be stale or absent.
 */
export const LOADER_SOURCE_PATH = resolve('src/lib/loader.ts');

/**
 * Creates a mock Timings object for testing.
 */
export const create_mock_timings = (): Timings =>
	({
		start: vi.fn(() => vi.fn())
	}) as unknown as Timings;

/**
 * Creates a mock `ModuleGraph` whose queries throw, for tests that never use it.
 */
export const create_mock_module_graph = (): ModuleGraph => ({
	modules: () => Promise.reject(Error('mock ModuleGraph: modules not implemented')),
	dependents: () => Promise.reject(Error('mock ModuleGraph: dependents not implemented')),
	watch: () => Promise.reject(Error('mock ModuleGraph: watch not implemented')),
	close: vi.fn(() => Promise.resolve())
});

/**
 * Creates a mock ParsedSvelteConfig for testing.
 */
export const create_mock_svelte_config = (): ParsedSvelteConfig =>
	({
		src_path: 'src',
		routes_path: 'src/routes'
	}) as ParsedSvelteConfig;

/**
 * Creates a mock TaskContext for testing.
 * Combines all the mock helpers into a complete task context.
 * Uses a synchronous config with a default hash for simplicity.
 *
 * @param args - Args to use in the context (can be partial if defaults are provided)
 * @param config_overrides - Partial config to override defaults
 * @param defaults - Default args to merge with provided args
 */
export const create_mock_task_context = <TArgs extends object = any>(
	args: Partial<TArgs> = {},
	config_overrides: Partial<GroConfig> = {},
	defaults?: TArgs
): TaskContext<TArgs> => ({
	task_args: (defaults ? { ...defaults, ...args } : args) as TArgs,
	config: {
		plugins: () => [],
		map_package_json: null,
		search_filters: [],
		build_cache_config_hash: 'test_hash',
		...config_overrides
	} as GroConfig,
	svelte_config: Promise.resolve(create_mock_svelte_config()),
	graph: create_mock_module_graph(),
	log: create_mock_logger(),
	timings: create_mock_timings(),
	gro: TEST_GRO_PACKAGE,
	invoke_task: vi.fn(),
	implicit_rest_args: [],
	raw_rest_args: []
});

/**
 * What `create_mock_plugin_context` builds a plugin's context from.
 */
export interface MockPluginContextOptions {
	dev: boolean;
	watch: boolean;
	/**
	 * The handles the test gives the plugin, each one left out throwing on any use.
	 */
	host?: Partial<PluginHost>;
	/**
	 * The CLI args from the first `--` on.
	 */
	raw_rest_args?: ReadonlyArray<string>;
}

/**
 * A plugin's context over `create_mock_task_context`, with the test's handles as its `host`,
 * so a plugin reaches only what the test gives it.
 */
export const create_mock_plugin_context = (options: MockPluginContextOptions): PluginContext => {
	const { dev, watch, host = {}, raw_rest_args = [] } = options;
	const create_stub = <T extends object>(label: string): T =>
		new Proxy({} as T, {
			get: (_target, key) => {
				throw Error(`the mock plugin host's \`${label}\` was used: ${String(key)}`);
			}
		});
	return {
		...create_mock_task_context(),
		raw_rest_args,
		dev,
		watch,
		host: {
			fs_paths: host.fs_paths ?? create_stub('fs_paths'),
			env: host.env ?? create_stub('env'),
			spawn: host.spawn ?? create_stub('spawn'),
			import_module: host.import_module ?? create_stub('import_module'),
			get loader_path(): string {
				if (host.loader_path === undefined) {
					throw Error("the mock plugin host's `loader_path` was used");
				}
				return host.loader_path;
			}
		}
	};
};

/**
 * Asserts that `value`, returned from an async function while `Object.prototype.then` is
 * polluted, resolves to itself, the polluted `then` never called.
 */
export const assert_awaits_as_itself = async (value: object): Promise<void> => {
	let calls = 0;
	(Object.prototype as any).then = function (resolve: (value: string) => void) {
		calls++;
		resolve('forged');
	};
	let returned: Promise<unknown>;
	try {
		// the return resolves the promise, which reads `then` synchronously, so nothing awaits
		// while it's polluted
		returned = (async () => value)();
	} finally {
		delete (Object.prototype as any).then;
	}
	assert.strictEqual(await returned, value);
	assert.strictEqual(calls, 0);
};

/**
 * Asserts that `fn` throws or rejects with a `TypeError`.
 * @param label - names the case in the failure message
 */
export const assert_rejects_type = async (
	fn: () => Promise<unknown>,
	label = ''
): Promise<void> => {
	try {
		await fn();
	} catch (error) {
		assert.instanceOf(error, TypeError, label);
		return;
	}
	assert.fail(`expected a TypeError ${label}`);
};
