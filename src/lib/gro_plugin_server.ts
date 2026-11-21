import type { Plugin } from './plugin.ts';
import { is_spawn_error } from './spawn_error.ts';
import { load_env_file } from './env_files.ts';
import { spawn_with_loader_args } from './gro_helpers.ts';
import { TaskError } from './task_error.ts';
import { start_watched_server, type WatchedServer } from './internal/server_restarts.ts';
import { format_error_detail } from './error_detail.ts';
import { posix_resolve } from './internal/posix_path.ts';

const LABEL = '[gro_plugin_server]';

/**
 * Options for `gro_plugin_server`.
 */
export interface GroPluginServerOptions {
	/**
	 * The server's entry module, relative to the project root, run as `gro run` runs a file.
	 */
	entry: string;
	/**
	 * The env file whose vars the server gets, absolute or relative to the project root,
	 * beneath the host's environment,
	 * so a var the host sets keeps the host's value. A missing file warns and loads nothing,
	 * and one that doesn't parse (`parse_env_file`) fails setup. `null` loads none.
	 * @default '.env.development'
	 */
	env_file?: string | null;
	/**
	 * Vars for the server over everything else, the host's environment and the env file included,
	 * like `PORT` and `HOST`. `NODE_ENV` is `development` unless this sets it.
	 * @default {}
	 */
	env?: Record<string, string>;
	/**
	 * Whether a change to the entry or anything it imports restarts the server.
	 * `gro dev --no-watch` starts no server, since it tears down right after setup.
	 * @default true
	 */
	watch?: boolean;
}

/**
 * Runs a server during development, beside the Vite dev server,
 * restarting it when its entry or anything the entry imports changes, per the module graph.
 * The entry runs with Gro's loader, as `gro run` runs a file, in its own process,
 * with the host's whole environment, since a server reads a set Gro can't know.
 * Only modules the graph tracks restart it: those under `watch.dir`, default `src/`,
 * reached by path or `#` subpath imports, not a package's files reached by bare specifiers.
 * Started at setup, only by `gro dev` in watch mode, and stopped at teardown.
 */
export const gro_plugin_server = (options: GroPluginServerOptions): Plugin => {
	const { entry, env_file = '.env.development', env: extra_env = {}, watch = true } = options;

	let server: WatchedServer | undefined;

	return {
		name: 'gro_plugin_server',
		setup: async ({ dev, watch: task_watch, log, config, graph, host }) => {
			if (!dev) return;
			// `gro dev --no-watch` tears down right after setup, so a server would only start to stop
			if (!task_watch) {
				log.info(`${LABEL} not starting ${entry} without watch`);
				return;
			}

			const { fs_paths, env, spawn, loader_path } = host;
			const root = fs_paths.project.path;
			const entry_file = await fs_paths.open_path(entry);
			if (!(await entry_file.dir.exists(entry_file.path))) {
				throw new TaskError(`${LABEL} cannot find the server entry: ${entry}`);
			}
			const entry_id = posix_resolve(root, entry);

			if (watch) {
				const { dir, filter } = config.watch;
				if (!entry_id.startsWith(dir + '/') || (filter !== null && !filter(entry_id, false))) {
					log.warn(
						`${LABEL} ${entry} is outside watch.dir ${dir} or rejected by watch.filter,` +
							" so the graph doesn't watch it and a change to it restarts nothing"
					);
				}
			}

			// the host's values win over the file's, as a dotenv loader leaves set vars alone
			const file_env: Record<string, string> = {};
			if (env_file !== null) {
				const loaded = await load_env_file(fs_paths, env_file);
				if (loaded === null) {
					log.warn(`${LABEL} env file not found, loading none: ${env_file}`);
				} else {
					for (const [name, value] of Object.entries(loaded)) {
						if (env.get(name) === undefined) file_env[name] = value;
					}
				}
			}
			const child_env = { ...file_env, NODE_ENV: 'development', ...extra_env };

			log.info(`${LABEL} starting ${entry}${watch ? ', restarting on changes' : ''}`);
			try {
				// pinned here, so every restart runs this `node` with this environment
				const node = spawn.program('node', { pass_env: 'all', env: child_env });
				const args = spawn_with_loader_args(loader_path, entry, [], child_env.NODE_ENV);
				server = await start_watched_server({
					program: node,
					args,
					entry_id,
					graph: watch ? graph : null,
					log,
					label: LABEL,
					cwd: root
				});
			} catch (error) {
				if (is_spawn_error(error)) {
					throw new TaskError(format_error_detail(error), { cause: error });
				}
				throw error;
			}
		},
		teardown: async ({ log }) => {
			if (!server) return;
			const s = server;
			server = undefined;
			log.info(`${LABEL} stopping ${entry}`);
			await s.stop();
		}
	};
};
