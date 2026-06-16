import {attach_process_error_handler} from '@fuzdev/fuz_util/process.ts';
import {configure_print_colors} from '@fuzdev/fuz_util/print.ts';

import {invoke_task} from './invoke_task.ts';
import {to_task_args} from './args.ts';
import {load_gro_config} from './gro_config.ts';
import {sveltekit_sync_if_obviously_needed} from './sveltekit_helpers.ts';

/*

This module invokes the Gro CLI which in turn invokes tasks.
Tasks are the CLI's primary concept.
To learn more about them, see `src/docs/task.md`.

When the CLI is invoked it passes the first CLI arg as `task_name` to `invoke_task`,
and the rest of the args are forwarded to the task's `run` function.

*/

// handle uncaught errors
attach_process_error_handler({
	to_error_label: (err) => (err.constructor.name === 'TaskError' ? 'TaskError' : null),
	map_error_text: (err) => (err.constructor.name === 'SilentError' ? '' : null),
});

if (!process.env.NO_COLOR) {
	const {styleText} = await import('node:util');
	configure_print_colors(styleText);
}

await sveltekit_sync_if_obviously_needed();

const {task_name, args} = to_task_args();
await invoke_task(task_name, args, await load_gro_config());
