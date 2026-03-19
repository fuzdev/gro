/**
 * Gro plugin for running a Deno server alongside the Vite dev server.
 *
 * Unlike `gro_plugin_server`, this doesn't use esbuild bundling since
 * Deno can run TypeScript directly with JSR imports.
 *
 * @module
 */

import {type RestartableProcess, spawn_restartable_process} from '@fuzdev/fuz_util/process.js';
import type {Plugin} from './plugin.js';

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
	 * @default true in `dev`
	 */
	watch?: boolean;

	/**
	 * Env file to load via Deno's --env flag.
	 * Set to `null` to disable env file loading.
	 * @default '.env.development'
	 */
	env_file?: string | null;

	/**
	 * Additional environment variables to pass to the child process.
	 * Always merged with the parent process.env (for PATH, HOME, etc.).
	 * PORT, HOST, and NODE_ENV are set automatically from the plugin options.
	 * @default {}
	 */
	env?: Record<string, string>;
}

/**
 * Creates a gro plugin that runs a Deno server during development.
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
		env: extra_env = {},
	} = options;

	let server_process: RestartableProcess | undefined;

	return {
		name: 'gro_plugin_deno_server',
		setup: async ({dev, log}) => {
			if (!dev) return; // Only run in dev mode

			const should_watch = watch ?? dev;

			log.info(`[gro_plugin_deno_server] starting Deno server: ${entry} on http://${host}:${port}`);

			const args = ['run', ...permissions, ...flags];

			if (env_file != null) {
				args.push(`--env=${env_file}`);
			}

			if (should_watch) {
				args.push('--watch');
			}

			args.push(entry);

			// Extend process.env rather than replacing it (need PATH, HOME, etc.)
			server_process = spawn_restartable_process('deno', args, {
				env: {
					...process.env,
					PORT: String(port),
					HOST: host,
					NODE_ENV: 'development',
					...extra_env,
				},
			} as object);

			// Wait for spawn to complete
			await server_process.spawned;
			log.info('[gro_plugin_deno_server] deno server started');
		},
		teardown: async ({log}) => {
			if (server_process) {
				log.info('[gro_plugin_deno_server] stopping Deno server');
				await server_process.kill();
				server_process = undefined;
			}
		},
	};
};
