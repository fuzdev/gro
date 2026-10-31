import { print_spawn_result } from '@fuzdev/fuz_util/process.ts';
import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';

import { has_forwarded_flag, to_forwarded_argv } from './args.ts';
import { configure_colored_output_with_path_replacement } from './child_process_logging.ts';
import { find_cli, spawn_cli, spawn_cli_process } from './cli.ts';
import { SVELTE_CHECK_CLI } from './constants.ts';
import { paths } from './paths.ts';
import { sveltekit_sync_if_available } from './sveltekit_helpers.ts';
import { TaskError, type Task } from './task.ts';

/** @nodocs */
export const Args = s.object({
	svelte_check_cli: s.with_default(
		meta(s.string, { description: 'the svelte-check CLI to use' }),
		SVELTE_CHECK_CLI
	),
	typescript_cli: s.with_default(
		meta(s.string, { description: 'the TypeScript CLI to use as a fallback to svelte-check' }),
		'tsc'
	),
	path_replacement: s.with_default(
		meta(s.string, { description: 'replacement string for current working directory in output' }),
		'.'
	),
	cwd: s.with_default(meta(s.string, { description: 'current working directory' }), paths.root)
});
export type Args = s.Infer<typeof Args>;

/** @nodocs */
export const task: Task<Args> = {
	summary: 'run svelte-check or tsc on the project without emitting any files',
	Args,
	run: async ({ args, log }): Promise<void> => {
		const { svelte_check_cli, typescript_cli, path_replacement, cwd } = args;

		await sveltekit_sync_if_available();

		// Prefer svelte-check if available.
		const found_svelte_check_cli = await find_cli(svelte_check_cli);
		if (found_svelte_check_cli) {
			const forwarded_argv = to_forwarded_argv(svelte_check_cli);
			const spawned = await spawn_cli_process(found_svelte_check_cli, forwarded_argv, undefined, {
				stdio: ['inherit', 'pipe', 'pipe'],
				env: { ...process.env, FORCE_COLOR: '1' } // Needed for colors (maybe make an option)
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
			const forwarded_argv = to_forwarded_argv(typescript_cli);
			// tsc's flags are case-insensitive, so a forwarded `--noemit` overrides too,
			// and `--noEmit` goes last because `--build` must be tsc's first arg
			const tsc_argv = has_forwarded_flag(forwarded_argv, 'noEmit', true)
				? forwarded_argv
				: [...forwarded_argv, '--noEmit'];
			const tsc_result = await spawn_cli(found_typescript_cli, tsc_argv, log);
			if (!tsc_result?.ok) {
				throw new TaskError(`Failed to typecheck. ${print_spawn_result(tsc_result!)}`);
			}
			return;
		}

		throw new TaskError(
			`Failed to typecheck because neither \`${svelte_check_cli}\` nor \`${
				typescript_cli
			}\` was found`
		);
	}
};
