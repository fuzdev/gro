import {args_serialize} from '@fuzdev/fuz_util/args.ts';
import {fs_exists} from '@fuzdev/fuz_util/fs.ts';
import {spawn_result_to_message} from '@fuzdev/fuz_util/process.ts';
import {styleText as st} from 'node:util';
import {z} from 'zod';

import {to_implicit_forwarded_args} from './args.ts';
import {resolve_gro_module_path, spawn_with_loader} from './gro_helpers.ts';
import {TaskError, type Task} from './task.ts';

/**
 * Runs a TypeScript file with Gro's loader, forwarding all args to the script.
 * Useful for scripts that need SvelteKit shims ($lib, $env, etc).
 *
 * @module
 */

/** @nodocs */
export const Args = z
	.object({
		_: z.array(z.string()).meta({description: 'the file path to run'}).default([]),
	})
	.catchall(
		z.union([
			z.string(),
			z.number(),
			z.boolean(),
			z.array(z.union([z.string(), z.number(), z.boolean()])),
		]),
	);
export type Args = z.infer<typeof Args>;

/** @nodocs */
export const task: Task<Args> = {
	summary: 'execute a file with the loader, like `node` but works for TypeScript',
	Args,
	run: async ({args, log}) => {
		const {_, ...forwarded_args} = args;
		const [path, ...positional_argv] = _;

		if (!path) {
			log.info(st('green', '\n\nUsage: ') + st('cyan', 'gro run path/to/file.ts [...args]\n'));
			return;
		}

		if (!(await fs_exists(path))) {
			throw new TaskError('Cannot find file to run at path: ' + path);
		}

		// Get args after `--` without requiring a command name.
		// This allows `gro run script.ts -- --help` to pass --help to the script.
		const implicit_args = to_implicit_forwarded_args();

		// Reconstruct argv: positional args + explicit named args + implicit args after --
		const named_argv = args_serialize({...forwarded_args, ...implicit_args});
		const full_argv = [...positional_argv, ...named_argv];

		const loader_path = resolve_gro_module_path('loader.js');

		const spawned = await spawn_with_loader(loader_path, path, full_argv);
		if (!spawned.ok) {
			throw new TaskError(`\`gro run ${path}\` failed: ${spawn_result_to_message(spawned)}`);
		}
	},
};
