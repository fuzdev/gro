import { args_serialize } from '@fuzdev/fuz_util/args.ts';
import { styleText as st } from 'node:util';
import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';

import { resolve_gro_module_path, spawn_with_loader, to_signal_exit_code } from './gro_helpers.ts';
import { process } from './process.ts';
import { TaskError, type Task } from './task.ts';
import { fs } from './fs.ts';
import { create_fs_paths } from './fs_paths.ts';
import { spawn, type SpawnResult } from './spawn.ts';
import { is_spawn_error } from './spawn_error.ts';
import { spawn_result_is_ok, to_spawn_result_message } from './cli.ts';

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
	run: async ({ task_args, log }) => {
		const { _, ...named_args } = task_args;
		const [path, ...positional_argv] = _;

		if (!path) {
			log.info(st('green', '\n\nUsage: ') + st('cyan', 'gro run path/to/file.ts [...args]\n'));
			return;
		}

		const file = await create_fs_paths(fs).open_path(path);
		if (!(await file.dir.exists(file.path))) {
			throw new TaskError('Cannot find file to run at path: ' + path);
		}

		// the CLI's verbatim script args are all positionals, and a programmatic call's named args follow
		const full_argv = [...positional_argv, ...args_serialize(named_args)];

		const loader_path = resolve_gro_module_path('loader.js');

		let spawned: SpawnResult;
		try {
			// the script is the user's, so it gets the whole environment
			const node = spawn.program('node', { pass_env: 'all' });
			spawned = await spawn_with_loader(
				node,
				loader_path,
				path,
				full_argv,
				process.env.get('NODE_ENV')
			);
		} catch (error) {
			if (is_spawn_error(error)) throw new TaskError(error.message, { cause: error });
			throw error;
		}
		if (!spawn_result_is_ok(spawned)) {
			// the CLI exits with the script's own code, like `node`,
			// or for a signal `128` plus its number, like a shell (`143` for `SIGTERM`)
			throw new TaskError(`\`gro run ${path}\` failed: ${to_spawn_result_message(spawned)}`, {
				exit_code:
					spawned.kind === 'exited'
						? spawned.code
						: spawned.kind === 'signaled'
							? to_signal_exit_code(spawned.signal)
							: undefined
			});
		}
	}
};
