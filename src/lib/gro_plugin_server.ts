import {spawn_restartable_process, type RestartableProcess} from '@fuzdev/fuz_util/process.js';
import * as esbuild from 'esbuild';
import type {Config as SvelteConfig} from '@sveltejs/kit';
import {join, resolve} from 'node:path';
import {identity} from '@fuzdev/fuz_util/function.js';
import {strip_before, strip_end} from '@fuzdev/fuz_util/string.js';
import type {Result} from '@fuzdev/fuz_util/result.js';
import {fs_exists} from '@fuzdev/fuz_util/fs.js';
import {throttle} from '@fuzdev/fuz_util/throttle.js';
import type {PathId} from '@fuzdev/fuz_util/path.js';

import type {Plugin} from './plugin.ts';
import {base_path_to_path_id, LIB_DIRNAME, paths} from './paths.ts';
import {GRO_DEV_DIRNAME, SERVER_DIST_PATH} from './constants.ts';
import {parse_svelte_config, default_svelte_config} from './svelte_config.ts';
import {esbuild_plugin_sveltekit_shim_app} from './esbuild_plugin_sveltekit_shim_app.ts';
import {esbuild_plugin_sveltekit_shim_env} from './esbuild_plugin_sveltekit_shim_env.ts';
import {print_build_result, to_define_import_meta_env} from './esbuild_helpers.ts';
import {esbuild_plugin_sveltekit_shim_alias} from './esbuild_plugin_sveltekit_shim_alias.ts';
import {esbuild_plugin_external_worker} from './esbuild_plugin_external_worker.ts';
import {esbuild_plugin_sveltekit_local_imports} from './esbuild_plugin_sveltekit_local_imports.ts';
import {esbuild_plugin_svelte} from './esbuild_plugin_svelte.ts';

// TODO sourcemap as a hoisted option? disable for production by default - or like `outpaths`, passed a `dev` param

export const SERVER_SOURCE_ID = base_path_to_path_id(LIB_DIRNAME + '/server/server.ts');

export const has_server = async (
	path = SERVER_SOURCE_ID,
): Promise<Result<object, {message: string}>> => {
	if (!(await fs_exists(path))) {
		return {ok: false, message: `no server file found at ${path}`};
	}
	return {ok: true};
};

export interface GroPluginServerOptions {
	/**
	 * same as esbuild's `entryPoints`
	 */
	entry_points?: Array<string>;
	/**
	 * @default cwd
	 */
	dir?: string;
	/**
	 * Returns the `Outpaths` given a `dev` param.
	 * Decoupling this from plugin creation allows it to be created generically,
	 * so the build and dev tasks can be the source of truth for `dev`.
	 */
	outpaths?: CreateOutpaths;
	/**
	 * @default ```SvelteKit's `.env`, `.env.development`, and `.env.production````
	 */
	env_files?: Array<string>;
	/**
	 * @default process.env
	 */
	ambient_env?: Record<string, string>;
	/**
	 * @default ```loaded from `${cwd}/${SVELTE_CONFIG_FILENAME}````
	 */
	svelte_config?: SvelteConfig;
	/**
	 * @default 'esnext'
	 */
	target?: string;
	/**
	 * Optionally map the esbuild options.
	 * @default `identity`
	 */
	esbuild_build_options?: (base_options: esbuild.BuildOptions) => esbuild.BuildOptions;
	/**
	 * Milliseconds to throttle rebuilds.
	 * Should be longer than it takes to build to avoid backpressure.
	 * @default 1000
	 */
	rebuild_throttle_delay?: number; // TODO could detect the backpressure problem and at least warn, shouldn't be a big deal
	/**
	 * The CLI command to run the server, like `'node'` or `'bun'` or `'deno'`.
	 * Receives the path to the server js file as its argument.
	 * @default 'node'
	 */
	cli_command?: string;
	/**
	 * Whether to run the server or not after building.
	 * @default `dev`
	 */
	run?: boolean;
}

export interface Outpaths {
	/**
	 * @default '.gro/dev' or 'dist_server'
	 */
	outdir: string;
	/**
	 * @default 'src/lib'
	 */
	outbase: string;
	/**
	 * @default 'server.js'
	 */
	outname: string;
}

export type CreateOutpaths = (dev: boolean) => Outpaths;

export const gro_plugin_server = ({
	entry_points = [SERVER_SOURCE_ID],
	dir = process.cwd(),
	outpaths = (dev) => ({
		outdir: join(dir, dev ? GRO_DEV_DIRNAME : SERVER_DIST_PATH),
		outbase: paths.lib,
		outname: 'server/server.js',
	}),
	env_files,
	ambient_env,
	svelte_config,
	target = 'esnext',
	esbuild_build_options = identity,
	rebuild_throttle_delay = 1000,
	cli_command,
	run, // `dev` default is not available in this scope
}: GroPluginServerOptions = {}): Plugin => {
	let build_ctx: esbuild.BuildContext | undefined;
	let cleanup_watch: (() => void) | undefined;
	let server_process: RestartableProcess | undefined;
	let deps: Set<PathId> | undefined;

	return {
		name: 'gro_plugin_server',
		setup: async ({dev, watch, timings, log, config, filer}) => {
			const parsed_svelte_config =
				!svelte_config && strip_end(dir, '/') === process.cwd()
					? default_svelte_config
					: await parse_svelte_config({
							dir_or_config: svelte_config ?? dir,
							config_filename: config.svelte_config_filename,
						});
			const {
				alias,
				base_url,
				assets_url,
				env_dir,
				private_prefix,
				public_prefix,
				svelte_compile_options,
				svelte_compile_module_options,
				svelte_preprocessors,
			} = parsed_svelte_config;

			const {outbase, outdir, outname} = outpaths(dev);

			const server_outpath = join(outdir, outname);

			const timing_to_esbuild_create_context = timings.start('create build context');

			const build_options = esbuild_build_options({
				outdir,
				outbase,
				format: 'esm',
				platform: 'node',
				packages: 'external',
				bundle: true,
				target,
				metafile: watch,
			});

			build_ctx = await esbuild.context({
				entryPoints: entry_points.map((path) => resolve(dir, path)),
				plugins: [
					esbuild_plugin_sveltekit_shim_app({dev, base_url, assets_url}),
					esbuild_plugin_sveltekit_shim_env({
						dev,
						public_prefix,
						private_prefix,
						env_dir,
						env_files,
						ambient_env,
					}),
					esbuild_plugin_sveltekit_shim_alias({dir, alias}),
					esbuild_plugin_external_worker({
						dev,
						build_options,
						dir,
						svelte_compile_options,
						svelte_compile_module_options,
						svelte_preprocessors,
						alias,
						base_url,
						public_prefix,
						private_prefix,
						env_dir,
						env_files,
						ambient_env,
						log,
					}),
					esbuild_plugin_svelte({
						dev,
						base_url,
						dir,
						svelte_compile_options,
						svelte_compile_module_options,
						svelte_preprocessors,
					}),
					esbuild_plugin_sveltekit_local_imports(),
				],
				define: to_define_import_meta_env(dev, base_url),
				...build_options,
			});

			timing_to_esbuild_create_context();

			const rebuild = throttle(
				async () => {
					let build_result;
					try {
						build_result = await build_ctx!.rebuild();
					} catch (error) {
						log.error('[gro_plugin_server] build failed', error);
						return;
					}
					const {metafile} = build_result;
					if (!metafile) return;
					print_build_result(log, build_result);
					deps = parse_deps(metafile.inputs, dir);
					void server_process?.restart();
				},
				{delay: rebuild_throttle_delay},
			);

			await rebuild();

			if (watch) {
				cleanup_watch = await filer.watch((change) => {
					if (!deps?.has(change.path)) {
						return;
					}
					void rebuild();
				});
			}

			if (!(await fs_exists(server_outpath))) {
				throw Error(`Node server failed to start due to missing file: ${server_outpath}`);
			}

			if (run || dev) {
				const cli_args = [];
				if (dev) {
					cli_args.push('-C', 'development'); // same as `--conditions`
				}
				cli_args.push(server_outpath);
				server_process = spawn_restartable_process(cli_command ?? config.js_cli, cli_args);
			}
		},
		teardown: async () => {
			if (cleanup_watch) {
				cleanup_watch();
				cleanup_watch = undefined;
			}

			if (server_process) {
				const s = server_process; // avoid possible issue where a build is in progress, don't want to issue a restart, could be fixed upstream in `spawn_restartable_process`
				server_process = undefined;
				await s.kill();
			}

			if (build_ctx) {
				await build_ctx.dispose();
				build_ctx = undefined;
			}
		},
	};
};

/**
 * The esbuild metafile contains the paths in `entryPoints` relative to the `dir`
 * even though we're resolving them to absolute paths before passing them to esbuild,
 * so we resolve them here relative to the `dir`.
 */
const parse_deps = (metafile_inputs: Record<string, unknown>, dir: string): Set<string> => {
	const deps: Set<string> = new Set();
	for (const key in metafile_inputs) {
		deps.add(resolve(dir, strip_before(key, ':')));
	}
	return deps;
};
