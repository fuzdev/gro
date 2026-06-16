import {args_serialize} from '@fuzdev/fuz_util/args.ts';
import {print_spawn_result} from '@fuzdev/fuz_util/process.ts';
import {z} from 'zod';

import {to_forwarded_args} from './args.ts';
import {configure_colored_output_with_path_replacement} from './child_process_logging.ts';
import {find_cli, spawn_cli, spawn_cli_process} from './cli.ts';
import {SVELTE_CHECK_CLI} from './constants.ts';
import {paths} from './paths.ts';
import {sveltekit_sync_if_available} from './sveltekit_helpers.ts';
import {TaskError, type Task} from './task.ts';

/** @nodocs */
export const Args = z.strictObject({
	svelte_check_cli: z
		.string()
		.meta({description: 'the svelte-check CLI to use'})
		.default(SVELTE_CHECK_CLI),
	typescript_cli: z
		.string()
		.meta({description: 'the TypeScript CLI to use as a fallback to svelte-check'})
		.default('tsc'),
	path_replacement: z
		.string()
		.meta({description: 'replacement string for current working directory in output'})
		.default('.'),
	cwd: z.string().meta({description: 'current working directory'}).default(paths.root),
});
export type Args = z.infer<typeof Args>;

/** @nodocs */
export const task: Task<Args> = {
	summary: 'run svelte-check or tsc on the project without emitting any files',
	Args,
	run: async ({args, log}): Promise<void> => {
		const {svelte_check_cli, typescript_cli, path_replacement, cwd} = args;

		await sveltekit_sync_if_available();

		// Prefer svelte-check if available.
		const found_svelte_check_cli = await find_cli(svelte_check_cli);
		if (found_svelte_check_cli) {
			const serialized = args_serialize(to_forwarded_args(svelte_check_cli));
			const spawned = await spawn_cli_process(found_svelte_check_cli, serialized, undefined, {
				stdio: ['inherit', 'pipe', 'pipe'],
				env: {...process.env, FORCE_COLOR: '1'}, // Needed for colors (maybe make an option)
			});

			const svelte_check_process = spawned?.child;
			if (svelte_check_process) {
				// Configure process output with path replacement while preserving colors
				configure_colored_output_with_path_replacement(svelte_check_process, path_replacement, cwd);

				const svelte_check_result = await spawned.closed;

				if (!svelte_check_result.ok) {
					throw new TaskError(`Failed to typecheck. ${print_spawn_result(svelte_check_result)}`);
				}
			}

			return;
		}

		// Fall back to tsc.
		const found_typescript_cli = await find_cli(typescript_cli);
		if (found_typescript_cli) {
			const forwarded = to_forwarded_args(typescript_cli);
			if (!forwarded.noEmit) forwarded.noEmit = true;
			const serialized = args_serialize(forwarded);
			const svelte_check_result = await spawn_cli(found_typescript_cli, serialized, log);
			if (!svelte_check_result?.ok) {
				throw new TaskError(`Failed to typecheck. ${print_spawn_result(svelte_check_result!)}`);
			}
			return;
		}

		throw new TaskError(
			`Failed to typecheck because neither \`${svelte_check_cli}\` nor \`${
				typescript_cli
			}\` was found`,
		);
	},
};
