import { to_forwarded_argv, to_raw_rest_args } from './task_args.ts';
import { args } from './args.ts';
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
import { TaskError } from './task_error.ts';

export const gro_plugin_sveltekit_app = (): Plugin => {
	let sveltekit_process: SpawnChild | undefined = undefined;
	return {
		name: 'gro_plugin_sveltekit_app',
		setup: async ({ dev, watch, log }) => {
			const found_vite_cli = find_cli(spawn, VITE_CLI, { pass_env: 'all' });
			if (!found_vite_cli) {
				throw new TaskError(
					`Failed to find Vite CLI \`${VITE_CLI}\`, do you need to run \`npm i\`?`
				);
			}
			if (dev) {
				// `vite dev` in development mode
				if (watch) {
					const serialized_args = ['dev', ...to_forwarded_argv(VITE_CLI, to_raw_rest_args(args))];
					sveltekit_process = await spawn_cli_process(found_vite_cli, serialized_args, log);
				} else {
					log.debug(
						`the SvelteKit app plugin is loaded but will not output anything` +
							' because `dev` is true and `watch` is false'
					);
				}
			} else {
				// `vite build` in production mode
				const serialized_args = ['build', ...to_forwarded_argv(VITE_CLI, to_raw_rest_args(args))];
				const result = await spawn_cli(found_vite_cli, serialized_args, log);
				if (!spawn_result_is_ok(result)) {
					throw new TaskError(`${VITE_CLI} build failed: ${to_spawn_result_message(result)}`);
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
