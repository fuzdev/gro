import { configure_print_colors } from '@fuzdev/fuz_util/print.ts';
import { styleText } from 'node:util';

import { gro_main, log_cli_error, to_exit_code } from './gro_main.ts';
import { fs } from './fs.ts';
import { spawn } from './spawn.ts';
import { create_fs_paths } from './fs_paths.ts';
import { cwd } from './cwd.ts';
import { args } from './args.ts';
import { env } from './env.ts';
import { exit, set_exit_code } from './exit.ts';
import { stderr } from './stderr.ts';
import { stdout } from './stdout.ts';
import { listen_to_node_process } from './internal/node_cli_process.ts';
import { import_module_node } from './internal/node_import_module.ts';
import { create_node_platform } from './internal/node_platform.ts';

/*

This module is the Node Gro CLI's entry, its composition root:
the fs, env, spawn, and stream host modules, the project loader, and the platform enter here,
and `gro_main` (`gro_main.ts`) runs what the CLI args name with them.
Only the Node-only parts stay here: the listeners on the global `process`,
the colors of Node's `styleText`, and the exit's flush.

*/

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
	log_cli_error(err, origin);
	void flush_output().finally(() => exit(to_exit_code(err)));
});

if (!env.get('NO_COLOR')) {
	configure_print_colors(styleText);
}

const fs_paths = create_fs_paths(fs, { cwd: cwd(), home: env.get('HOME') });

await gro_main({
	args,
	fs_paths,
	host_root: { env, spawn, stdout, stderr },
	import_module: import_module_node,
	platform: await create_node_platform(fs_paths)
});
