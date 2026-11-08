import { to_forwarded_argv } from './args.ts';
import {
	find_cli,
	spawn_cli,
	spawn_cli_process,
	spawn_result_is_ok,
	to_spawn_result_message
} from './cli.ts';
import { VITE_CLI } from './constants.ts';
import type { Plugin } from './plugin.ts';
import { spawn, type SpawnChild } from './spawn.ts';
import { TaskError } from './task.ts';

export interface GroPluginSveltekitAppOptions {
	/**
	 * The Vite CLI to use.
	 */
	vite_cli?: string;
}

export const gro_plugin_sveltekit_app = ({
	vite_cli = VITE_CLI
}: GroPluginSveltekitAppOptions = {}): Plugin => {
	let sveltekit_process: SpawnChild | undefined = undefined;
	return {
		name: 'gro_plugin_sveltekit_app',
		setup: async ({ dev, watch, log, config }) => {
			const found_vite_cli = find_cli(spawn, vite_cli, { pass_env: 'all' });
			if (!found_vite_cli) {
				throw new TaskError(
					`Failed to find Vite CLI \`${vite_cli}\`, do you need to run \`${config.pm_cli} i\`?`
				);
			}
			if (dev) {
				// `vite dev` in development mode
				if (watch) {
					const serialized_args = ['dev', ...to_forwarded_argv(vite_cli)];
					sveltekit_process = await spawn_cli_process(found_vite_cli, serialized_args, log);
				} else {
					log.debug(
						`the SvelteKit app plugin is loaded but will not output anything` +
							' because `dev` is true and `watch` is false'
					);
				}
			} else {
				// `vite build` in production mode
				const serialized_args = ['build', ...to_forwarded_argv(vite_cli)];
				const result = await spawn_cli(found_vite_cli, serialized_args, log);
				if (!spawn_result_is_ok(result)) {
					throw new TaskError(`${vite_cli} build failed: ${to_spawn_result_message(result)}`);
				}
			}
		},
		teardown: async () => {
			if (sveltekit_process) {
				sveltekit_process.kill();
				await sveltekit_process.closed;
			}
		}
	};
};
