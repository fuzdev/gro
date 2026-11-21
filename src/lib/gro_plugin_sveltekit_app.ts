import { to_forwarded_argv } from './task_args.ts';
import {
	find_cli,
	spawn_cli,
	spawn_cli_process,
	spawn_result_is_ok,
	to_spawn_result_message
} from './cli.ts';
import { VITE_CLI } from './constants.ts';
import type { Plugin } from './plugin.ts';
import type { SpawnChild } from './spawn.ts';
import { TaskError } from './task_error.ts';
import { assert_no_forwarded_info_flag } from './internal/forwarded_info_flags.ts';

// Vite ignores a version flag after the command Gro gives it, so only help is refused
const to_vite_forwarded_argv = (raw_rest_args: ReadonlyArray<string>): Array<string> => {
	const forwarded_argv = to_forwarded_argv(VITE_CLI, raw_rest_args);
	assert_no_forwarded_info_flag(VITE_CLI, forwarded_argv, false);
	return forwarded_argv;
};

export const gro_plugin_sveltekit_app = (): Plugin => {
	let sveltekit_process: SpawnChild | undefined = undefined;
	return {
		name: 'gro_plugin_sveltekit_app',
		setup: async ({ dev, watch, log, raw_rest_args, host }) => {
			const found_vite_cli = find_cli(host.spawn, VITE_CLI, { pass_env: 'all' });
			if (!found_vite_cli) {
				throw new TaskError(
					`Failed to find Vite CLI \`${VITE_CLI}\`, do you need to run \`npm i\`?`
				);
			}
			if (dev) {
				// `vite dev` in development mode
				if (watch) {
					const serialized_args = ['dev', ...to_vite_forwarded_argv(raw_rest_args)];
					sveltekit_process = await spawn_cli_process(found_vite_cli, serialized_args, log);
				} else {
					log.debug(
						`the SvelteKit app plugin is loaded but will not output anything` +
							' because `dev` is true and `watch` is false'
					);
				}
			} else {
				// `vite build` in production mode
				const serialized_args = ['build', ...to_vite_forwarded_argv(raw_rest_args)];
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
