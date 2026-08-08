import { spawn_restartable_process, type RestartableProcess } from '@fuzdev/fuz_util/process.ts';
import * as esbuild from 'esbuild';
import type { Config as SvelteConfig } from '@sveltejs/kit';
import { join, resolve } from 'node:path';
import { identity } from '@fuzdev/fuz_util/function.ts';
import { strip_before } from '@fuzdev/fuz_util/string.ts';
import type { Result } from '@fuzdev/fuz_util/result.ts';
import { fs_exists } from '@fuzdev/fuz_util/fs.ts';
import { throttle } from '@fuzdev/fuz_util/throttle.ts';
import type { PathId } from '@fuzdev/fuz_util/path.ts';

import type { Plugin } from './plugin.ts';
import { paths } from './paths.ts';
import { GRO_DEV_DIRNAME, SERVER_DIST_PATH } from './constants.ts';
import { parse_svelte_config, load_default_svelte_config } from './svelte_config.ts';
import { esbuild_plugin_sveltekit_shim_app } from './esbuild_plugin_sveltekit_shim_app.ts';
import { esbuild_plugin_sveltekit_shim_env } from './esbuild_plugin_sveltekit_shim_env.ts';
import { print_build_result, to_define_import_meta_env } from './esbuild_helpers.ts';
import { esbuild_plugin_sveltekit_shim_alias } from './esbuild_plugin_sveltekit_shim_alias.ts';
import { esbuild_plugin_external_worker } from './esbuild_plugin_external_worker.ts';
import {
	esbuild_plugin_sveltekit_local_imports
} from './esbuild_plugin_sveltekit_local_imports.ts';
import { esbuild_plugin_svelte } from './esbuild_plugin_svelte.ts';

// TODO sourcemap as a hoisted option? disable for production by default - or like `outpaths`, passed a `dev` param

/**
 * The server entry point, relative to the project's lib directory.
 */
export const SERVER_SOURCE_PATH = 'server/server.ts';

/**
 * The server entry point of a project whose lib directory is `lib_path`.
 * Taken from the Svelte config's `files.lib` rather than the conventional `src/lib`,
 * so a project that moves its lib directory still has its server found and built.
 */
export const to_server_source_id = (lib_path: string): PathId =>
	join(paths.root, lib_path, SERVER_SOURCE_PATH);

/**
 * @param path - the server entry point to look for;
 * defaults to `to_server_source_id` of the Svelte config's `lib_path`
 */
export const has_server = async (path?: string): Promise<Result<object, { message: string }>> => {
	const final_path = path ?? to_server_source_id((await load_default_svelte_config()).lib_path);
	if (!(await fs_exists(final_path))) {
		return { ok: false, message: `no server file found at ${final_path}` };
	}
	return { ok: true };
};

export interface GroPluginServerOptions {
	/**
	 * same as esbuild's `entryPoints`
	 * @default ```[`to_server_source_id` of the Svelte config's `lib_path`]````
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
	 * @default `to_default_outpaths`
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
	 * An already-loaded Svelte config, to skip resolving the project's Vite config.
	 * @default ```resolved from the project's Vite config````
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
	 * @default ```the Svelte config's `lib_path`, so `src/lib` unless it's customized````
	 */
	outbase: string;
	/**
	 * @default 'server.js'
	 */
	outname: string;
}

export type CreateOutpaths = (dev: boolean) => Outpaths;

/**
 * The `Outpaths` used when the plugin is given none.
 * Takes `lib_dir` as a param rather than reading `paths.lib` so a customized `kit.files.lib`
 * is honored - resolving it is async, so it can't be a plugin-creation default.
 */
export const to_default_outpaths =
	(dir: string, lib_dir: string): CreateOutpaths =>
	(dev) => ({
		outdir: join(dir, dev ? GRO_DEV_DIRNAME : SERVER_DIST_PATH),
		outbase: lib_dir,
		outname: 'server/server.js'
	});

export const gro_plugin_server = ({
	entry_points,
	dir = process.cwd(),
	outpaths,
	env_files,
	ambient_env,
	svelte_config,
	target = 'esnext',
	esbuild_build_options = identity,
	rebuild_throttle_delay = 1000,
	cli_command,
	run // `dev` default is not available in this scope
}: GroPluginServerOptions = {}): Plugin => {
	let build_ctx: esbuild.BuildContext | undefined;
	let cleanup_watch: (() => void) | undefined;
	let server_process: RestartableProcess | undefined;
	let deps: Set<PathId> | undefined;

	return {
		name: 'gro_plugin_server',
		setup: async ({ dev, watch, timings, log, config, filer }) => {
			// `load_default_svelte_config` memoizes, so this shares the resolution
			// with the rest of the process. Note that it reads the cwd's config,
			// not `dir`'s - `dir` positions esbuild's output and alias resolution.
			const parsed_svelte_config = svelte_config
				? await parse_svelte_config({ svelte_config })
				: await load_default_svelte_config();
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
				lib_path
			} = parsed_svelte_config;

			// The entry point and `outbase` defaults land here rather than in the destructuring above
			// because they come from the Svelte config, which can only be read asynchronously.
			const lib_dir = join(paths.root, lib_path);
			const final_entry_points = entry_points ?? [join(lib_dir, SERVER_SOURCE_PATH)];

			const { outbase, outdir, outname } = (outpaths ?? to_default_outpaths(dir, lib_dir))(dev);

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
				metafile: watch
			});

			build_ctx = await esbuild.context({
				entryPoints: final_entry_points.map((path) => resolve(dir, path)),
				plugins: [
					esbuild_plugin_sveltekit_shim_app({ dev, base_url, assets_url }),
					esbuild_plugin_sveltekit_shim_env({
						dev,
						public_prefix,
						private_prefix,
						env_dir,
						env_files,
						ambient_env
					}),
					esbuild_plugin_sveltekit_shim_alias({ dir, alias }),
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
						log
					}),
					esbuild_plugin_svelte({
						dev,
						base_url,
						dir,
						svelte_compile_options,
						svelte_compile_module_options,
						svelte_preprocessors
					}),
					esbuild_plugin_sveltekit_local_imports()
				],
				define: to_define_import_meta_env(dev, base_url),
				...build_options
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
					const { metafile } = build_result;
					if (!metafile) return;
					print_build_result(log, build_result);
					deps = parse_deps(metafile.inputs, dir);
					void server_process?.restart();
				},
				{ delay: rebuild_throttle_delay }
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
		}
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
