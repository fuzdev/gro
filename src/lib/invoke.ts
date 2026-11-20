import { configure_print_colors } from '@fuzdev/fuz_util/print.ts';
import { Logger } from '@fuzdev/fuz_util/log.ts';
import { styleText as st } from 'node:util';

import { invoke_cli } from './invoke_task.ts';
import { to_raw_rest_args, to_task_args, type CliTaskArgs } from './task_args.ts';
import { ReportedError, is_reported_error, is_task_error } from './task_error.ts';
import { load_gro_config } from './gro_config.ts';
import { sveltekit_sync_if_obviously_needed } from './sveltekit_helpers.ts';
import { fs } from './fs.ts';
import { spawn } from './spawn.ts';
import { create_fs_paths } from './fs_paths.ts';
import { cwd } from './cwd.ts';
import { print_error_with_causes } from './internal/error_report.ts';
import { args } from './args.ts';
import { env } from './env.ts';
import { exit, set_exit_code } from './exit.ts';
import { stderr } from './stderr.ts';
import { stdout } from './stdout.ts';
import { listen_to_node_process } from './internal/node_cli_process.ts';
import { import_module_node } from './internal/node_import_module.ts';

/*

This module is the Gro CLI's entry, which invokes tasks, the CLI's primary concept.
To learn more about them, see `src/docs/task.md`.

`to_task_args` parses the CLI args into a `CliTaskArgs`, whose `task_name` is the first
positional arg, a builtin name or the `task` verb, with the path after `task` as its `task_path`.
`invoke_cli` runs what they name, taking `to_raw_rest_args`, the args from the first `--` on
verbatim, as its `raw_rest_args`; `gro run` has none, since every arg after its path is the script's.

*/

/**
 * The code the CLI exits with for an uncaught error:
 * a `TaskError`'s `exit_code`, like `gro run`'s script's own, when it's an integer 1–255
 * (a process exit code is a byte, so `256` would exit `0`)
 * and it's the uncaught error itself, as when thrown from `gro.config.ts` before any task runs,
 * or failed the task the CLI invoked (the `cause` of `invoke_task`'s `ReportedError`),
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

// how long an uncaught error's exit waits on a reader that isn't reading
const FLUSH_TIMEOUT_MS = 5_000;

// TODO: libuv clears `O_NONBLOCK` on the host's own stdio when it starts a child inheriting it,
// so after one a write past a pipe's buffer blocks in the kernel, where this timer can't reach

/**
 * Resolves once what's been written so far has been accepted,
 * as `exit` drops writes still queued for a slow reader on a pipe,
 * or after `FLUSH_TIMEOUT_MS`, so a reader that never reads can't hold the exit.
 */
const flush_output = async (): Promise<void> => {
	await Promise.race([
		Promise.all([stdout.write(''), stderr.write('')]),
		new Promise((resolve) => {
			setTimeout(resolve, FLUSH_TIMEOUT_MS).unref();
		})
	]);
};

// handle uncaught errors, the spawn module's exit hook ending any child still running
listen_to_node_process((err, origin) => {
	// so every exit path carries the code, even if the flush below never settles
	set_exit_code(to_exit_code(err));
	if (is_task_error(err)) {
		// a known failure, so its message with no stack
		new Logger('TaskError').error(st('red', err.message));
	} else if (!is_reported_error(err)) {
		// `print_error` reads properties, so a thrown `undefined` or `null` would throw here
		new Logger(origin).error(err == null ? `Unknown error: ${err}` : print_error_with_causes(err));
	}
	void flush_output().finally(() => exit(to_exit_code(err)));
});

if (!env.get('NO_COLOR')) {
	configure_print_colors(st);
}

// the CLI's composition root, where the fs, env, and spawn host modules and the project loader enter,
// the builtins getting what their grants name from the env and spawn roots
const fs_paths = create_fs_paths(fs, { cwd: cwd(), home: env.get('HOME') });

await sveltekit_sync_if_obviously_needed(fs_paths, spawn);

let task_args: CliTaskArgs;
try {
	task_args = to_task_args(args);
} catch (error) {
	if (!is_task_error(error)) throw error;
	new Logger('gro').error(st('red', error.message)); // a usage error, no stack trace
	throw new ReportedError();
}
await invoke_cli(
	fs_paths,
	task_args,
	await load_gro_config(fs_paths, import_module_node),
	to_raw_rest_args(args),
	import_module_node,
	{ env, spawn }
);
