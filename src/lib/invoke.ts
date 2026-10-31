import { attach_process_error_handler } from '@fuzdev/fuz_util/process.ts';
import { configure_print_colors } from '@fuzdev/fuz_util/print.ts';
import { Logger } from '@fuzdev/fuz_util/log.ts';
import { styleText as st } from 'node:util';

import { invoke_cli } from './invoke_task.ts';
import { to_task_args, type CliTaskArgs } from './args.ts';
import { SilentError, TaskError } from './task.ts';
import { load_gro_config } from './gro_config.ts';
import { sveltekit_sync_if_obviously_needed } from './sveltekit_helpers.ts';

/*

This module invokes the Gro CLI which in turn invokes tasks.
Tasks are the CLI's primary concept.
To learn more about them, see `src/docs/task.md`.

When the CLI is invoked it passes the first CLI arg as `task_name` to `invoke_cli`,
a builtin name or the `task` verb with a path, and the rest of the args are forwarded
to the task's `run` function.

*/

/**
 * The code the CLI exits with for an uncaught error:
 * a `TaskError`'s `exit_code`, like `gro run`'s script's own, when it's an integer 1–255
 * (a process exit code is a byte, so `256` would exit `0`)
 * and it failed the task the CLI invoked (the `cause` of `invoke_task`'s `SilentError`),
 * and `1` otherwise, including for a nested task's failure.
 */
const to_exit_code = (err: unknown): number => {
	const error = err as { exit_code?: unknown; cause?: { exit_code?: unknown } } | null | undefined;
	const exit_code = error?.exit_code ?? error?.cause?.exit_code;
	return typeof exit_code === 'number' &&
		Number.isInteger(exit_code) &&
		exit_code > 0 &&
		exit_code < 256
		? exit_code
		: 1;
};

// handle uncaught errors
attach_process_error_handler({
	to_error_label: (err) => (err.constructor.name === 'TaskError' ? 'TaskError' : null),
	map_error_text: (err) => (err.constructor.name === 'SilentError' ? '' : null),
	handle_error: (err) => process.exit(to_exit_code(err))
});

if (!process.env.NO_COLOR) {
	const { styleText } = await import('node:util');
	configure_print_colors(styleText);
}

await sveltekit_sync_if_obviously_needed();

let task_args: CliTaskArgs;
try {
	task_args = to_task_args();
} catch (error) {
	if (!(error instanceof TaskError)) throw error;
	new Logger('gro').error(st('red', error.message)); // a usage error, no stack trace
	throw new SilentError();
}
await invoke_cli(task_args, await load_gro_config());
