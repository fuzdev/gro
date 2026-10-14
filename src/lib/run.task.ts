import { args_serialize } from '@fuzdev/fuz_util/args.ts';
import { fs_exists } from '@fuzdev/fuz_util/fs.ts';
import { spawn_result_to_message } from '@fuzdev/fuz_util/process.ts';
import { styleText as st } from 'node:util';
import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';

import { to_implicit_forwarded_args } from './args.ts';
import { resolve_gro_module_path, spawn_with_loader } from './gro_helpers.ts';
import { TaskError, type Task } from './task.ts';

/**
 * Runs a TypeScript file with Gro's loader, forwarding all args to the script.
 * Useful for scripts that need SvelteKit shims (`$app/env`, `$app/paths`, etc).
 *
 * @module
 */

const ArgItem = s.union([s.string, s.number, s.boolean]);
const ArgValue = s.union([ArgItem, s.array(ArgItem)]);

// what `ArgValue` reports as expected, for the text of an arg that fails it
const ARG_VALUE_EXPECTED = 'string | number | boolean | array';

// the text of the first arg past `_` that isn't an `ArgValue`, keyed as `format_issue` writes a path
const to_invalid_arg_message = (args: Record<string, unknown>): string | undefined => {
	for (const key in args) {
		if (key === '_') continue;
		const value = args[key];
		if (s.is(ArgValue, value)) continue;
		const received = value === null ? 'null' : Array.isArray(value) ? 'array' : typeof value;
		return `${/^\w+$/.test(key) ? key : JSON.stringify(key)}: expected ${ARG_VALUE_EXPECTED}, received ${received}`;
	}
	return undefined;
};

// TODO `object_with_rest` in fuz_schema replaces this workaround: a `loose_object` passes
// its unknown keys through unchecked, so the `check` after it checks them, and the
// output and input types are asserted
/** @nodocs */
export const Args = s.pipe(
	s.loose_object({
		_: s.with_default_fn(meta(s.array(s.string), { description: 'the file path to run' }), () => [])
	}),
	s.message(
		s.check(
			(v: { _: Array<string> } & Record<string, unknown>) =>
				to_invalid_arg_message(v) === undefined,
			ARG_VALUE_EXPECTED
		),
		(_issue, v) => to_invalid_arg_message(v as Record<string, unknown>)!
	)
) as s.Schema<
	{ _: Array<string> } & Record<string, s.Infer<typeof ArgValue>>,
	{ _?: Array<string> } & Record<string, s.InferInput<typeof ArgValue>>
>;
export type Args = s.Infer<typeof Args>;

/** @nodocs */
export const task: Task<Args> = {
	summary: 'execute a file with the loader, like `node` but works for TypeScript',
	Args,
	run: async ({ args, log }) => {
		const { _, ...forwarded_args } = args;
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
		const named_argv = args_serialize({ ...forwarded_args, ...implicit_args });
		const full_argv = [...positional_argv, ...named_argv];

		const loader_path = resolve_gro_module_path('loader.js');

		const spawned = await spawn_with_loader(loader_path, path, full_argv);
		if (!spawned.ok) {
			throw new TaskError(`\`gro run ${path}\` failed: ${spawn_result_to_message(spawned)}`);
		}
	}
};
