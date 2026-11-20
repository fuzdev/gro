import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';
import type { Logger } from '@fuzdev/fuz_util/log.ts';

import { format_directory } from './format_directory.ts';
import type { FsPaths } from './fs_paths.ts';
import type { GroConfig } from './gro_config.ts';
import { create_paths } from './paths.ts';
import { TaskError } from './task_error.ts';
import type { BuiltinTask, BuiltinTaskGrants } from './builtin_host.ts';

/** @nodocs */
export const Args = s.object({
	_: s.optional(meta(s.array(s.string), { description: 'files or globs to format' })),
	check: s.with_default(
		meta(s.boolean, { description: 'exit with a nonzero code if any files are unformatted' }),
		false
	)
});
export type Args = s.Infer<typeof Args>;

const grants = {
	// reads the source files, and with no `--check` writes the ones it formats
	fs: { read: ['.'], write: ['.'] }
} as const satisfies BuiltinTaskGrants;

/** @nodocs */
export const task: BuiltinTask<Args, typeof grants> = {
	summary: 'format source files',
	Args,
	grants,
	run: ({ task_args, config, log, host }) =>
		run_format(
			task_args,
			{ search_filters: config.search_filters },
			{ fs_paths: host.fs_paths, log }
		)
};

/**
 * The data `run_format` takes beside its args.
 */
export interface FormatOptions {
	search_filters: GroConfig['search_filters'];
}

/**
 * What `run_format` reaches, picked from a builtin's `host` and context.
 */
export interface FormatDeps {
	fs_paths: FsPaths;
	log: Logger;
}

/**
 * Formats the project's source files, or checks them, `gro format`'s body.
 * @param args - the task's parsed args
 * @param options - the config's `search_filters`
 * @param deps - the project's boundary and the log
 */
export const run_format = async (
	args: Args,
	options: FormatOptions,
	deps: FormatDeps
): Promise<void> => {
	const { _: patterns, check } = args;
	const { fs_paths, log } = deps;
	const result = await format_directory(fs_paths, {
		log,
		dir: create_paths(fs_paths.project.path).source,
		check,
		filter: options.search_filters,
		patterns
	});
	if (!result.ok) {
		throw new TaskError(`Failed ${check ? 'formatting check' : 'to format'}.`);
	}
};
