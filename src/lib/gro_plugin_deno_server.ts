/**
 * Gro plugin for running a Deno server alongside the Vite dev server.
 *
 * It runs the entry unbundled, since Deno runs TypeScript directly with JSR imports.
 *
 * @module
 */

import type { Plugin } from './plugin.ts';
import { spawn } from './spawn.ts';
import { is_spawn_error } from './spawn_error.ts';
import { spawn_restartable, type SpawnRestartable } from './spawn_restartable.ts';
import { TaskError } from './task.ts';

export interface GroPluginDenoServerOptions {
	/**
	 * Entry point TypeScript file for the Deno server.
	 * @default 'src/lib/backend/server.ts'
	 */
	entry?: string;

	/**
	 * Port for the Deno server.
	 * @default 4041
	 */
	port?: number;

	/**
	 * Host for the Deno server.
	 * @default 'localhost'
	 */
	host?: string;

	/**
	 * Deno permissions to grant.
	 * @default ['--allow-net', '--allow-read', '--allow-env']
	 */
	permissions?: Array<string>;

	/**
	 * Additional deno run flags (e.g. --sloppy-imports, --no-check).
	 * @default []
	 */
	flags?: Array<string>;

	/**
	 * Whether to watch for file changes and restart.
	 * Uses Deno's built-in --watch flag.
	 * @default the task's `watch`, so `gro dev --no-watch` doesn't watch
	 */
	watch?: boolean;

	/**
	 * Env file to load via Deno's --env flag.
	 * Set to `null` to disable env file loading.
	 * @default '.env.development'
	 */
	env_file?: string | null;

	/**
	 * Additional environment variables to pass to the child process, over the host's whole
	 * environment, which a server with `--allow-env` reads from a set Gro can't know.
	 * PORT, HOST, and NODE_ENV are set automatically from the plugin options.
	 * An undefined value unsets the name, so the child doesn't get the host's value of it,
	 * except `NODE_V8_COVERAGE`, which Node copies into every child.
	 * @default {}
	 */
	env?: Record<string, string | undefined>;
}

/**
 * Splits env values into the ones to set and the names to unset, `undefined` meaning unset as
 * it did when spread over the host's environment, but for what Node copies in itself.
 */
const to_program_env = (
	env: Record<string, string | undefined>
): { env: Record<string, string>; omit_env: Array<string> } => {
	const defined: Record<string, string> = {};
	const omit_env: Array<string> = [];
	for (const [name, value] of Object.entries(env)) {
		if (value === undefined) omit_env.push(name);
		else defined[name] = value;
	}
	return { env: defined, omit_env };
};

/**
 * Creates a Gro plugin that runs a Deno server during development.
 * The server is started during setup and stopped on teardown.
 *
 * In dev mode, Vite should proxy API requests to this server.
 */
export const gro_plugin_deno_server = (options: GroPluginDenoServerOptions = {}): Plugin => {
	const {
		entry = 'src/lib/backend/server.ts',
		port = 4041,
		host = 'localhost',
		permissions = ['--allow-net', '--allow-read', '--allow-env'],
		flags = [],
		watch,
		env_file = '.env.development',
		env: extra_env = {}
	} = options;

	let server_process: SpawnRestartable | undefined;

	return {
		name: 'gro_plugin_deno_server',
		setup: async ({ dev, watch: task_watch, log }) => {
			if (!dev) return; // Only run in dev mode

			const should_watch = watch ?? task_watch;

			log.info(`[gro_plugin_deno_server] starting Deno server: ${entry} on http://${host}:${port}`);

			const args = ['run', ...permissions, ...flags];

			if (env_file != null) {
				args.push(`--env=${env_file}`);
			}

			if (should_watch) {
				args.push('--watch');
			}

			args.push(entry);

			// pinned here, so every start of the server runs this `deno` with this environment
			try {
				const deno = spawn.program('deno', {
					pass_env: 'all',
					...to_program_env({
						PORT: String(port),
						HOST: host,
						NODE_ENV: 'development',
						...extra_env
					})
				});
				server_process = await spawn_restartable(deno, args);
			} catch (error) {
				if (is_spawn_error(error)) throw new TaskError(error.message, { cause: error });
				throw error;
			}
			log.info('[gro_plugin_deno_server] deno server started');
		},
		teardown: async ({ log }) => {
			if (server_process) {
				log.info('[gro_plugin_deno_server] stopping Deno server');
				await server_process.stop();
				server_process = undefined;
			}
		}
	};
};
