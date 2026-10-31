import { args_serialize } from '@fuzdev/fuz_util/args.ts';
import { fs_exists } from '@fuzdev/fuz_util/fs.ts';
import { spawn_result_to_message } from '@fuzdev/fuz_util/process.ts';
import { constants } from 'node:os';
import { styleText as st } from 'node:util';
import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';

import { resolve_gro_module_path, spawn_with_loader } from './gro_helpers.ts';
import { TaskError, type Task } from './task.ts';

/**
 * Runs a TypeScript file with Gro's loader, forwarding all args to the script.
 * Useful for scripts that need SvelteKit shims (`$app/env`, `$app/paths`, etc).
 * From the CLI, `gro run <path> ...rest` passes everything after the path verbatim,
 * unparsed and including any `--`, like `node`, so `gro run x.ts --help` is the script's `--help`;
 * the CLI puts it all in the positionals.
 * Also like `node`, `gro run` prints no timings and exits with the script's exit code,
 * or for a script killed by a signal, `128` plus the signal's number, like a shell.
 * `invoke_task('run', {_: [path, ...positionals], ...named})` has no raw text,
 * so it serializes the named args after the positionals.
 *
 * @module
 */

const ArgItem = s.union([s.string, s.number, s.boolean]);
const ArgValue = s.union([s.string, s.number, s.boolean, s.array(ArgItem)]);

/** @nodocs */
export const Args = s.object_with_rest(
	{
		_: s.with_default_fn(meta(s.array(s.string), { description: 'the file path to run' }), () => [])
	},
	ArgValue
);
export type Args = s.Infer<typeof Args>;

/** @nodocs */
export const task: Task<Args> = {
	summary: 'execute a file with the loader, like `node` but works for TypeScript',
	Args,
	run: async ({ args, log }) => {
		const { _, ...named_args } = args;
		const [path, ...positional_argv] = _;

		if (!path) {
			log.info(st('green', '\n\nUsage: ') + st('cyan', 'gro run path/to/file.ts [...args]\n'));
			return;
		}

		if (!(await fs_exists(path))) {
			throw new TaskError('Cannot find file to run at path: ' + path);
		}

		// the CLI's verbatim script args are all positionals, and a programmatic call's named args follow
		const full_argv = [...positional_argv, ...args_serialize(named_args)];

		const loader_path = resolve_gro_module_path('loader.js');

		const spawned = await spawn_with_loader(loader_path, path, full_argv);
		if (!spawned.ok) {
			// the CLI exits with the script's own code, like `node`,
			// or for a signal `128` plus its number, like a shell (`143` for `SIGTERM`)
			throw new TaskError(`\`gro run ${path}\` failed: ${spawn_result_to_message(spawned)}`, {
				exit_code:
					spawned.kind === 'exited'
						? spawned.code
						: spawned.kind === 'signaled'
							? 128 + constants.signals[spawned.signal]
							: undefined
			});
		}
	}
};
