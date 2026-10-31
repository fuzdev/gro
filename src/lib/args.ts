import { styleText as st } from 'node:util';
import { argv_parse, type Args } from '@fuzdev/fuz_util/args.ts';

import {
	is_removed_builtin_prefix,
	is_task_path_input,
	TASK_VERB,
	TaskError,
	to_removed_builtin_prefix_message,
	to_task_path,
	to_task_path_without_verb_message
} from './task.ts';

/**
 * The task the CLI names and its args, parsed from `process.argv`.
 */
export interface CliTaskArgs {
	/**
	 * The first positional arg: a builtin name, `TASK_VERB`, or `''` for none.
	 */
	task_name: string;
	/**
	 * The path after `TASK_VERB`, `null` when absent or for any other `task_name`.
	 */
	task_path: string | null;
	args: Args;
}

/**
 * The builtin whose script args the CLI leaves unparsed, `gro run <path> ...rest`.
 */
const RUN_TASK_NAME = 'run';

/**
 * The flags `gro run` takes before the script path, Gro's own; any other is refused.
 */
const RUN_FLAGS: ReadonlySet<string> = new Set(['help']);

/**
 * The index in `argv` of the script path of `gro run [gro flags] <path> ...rest`,
 * the first arg after `run` that doesn't start with `-`, before any `--`,
 * so a flag before the path never takes a value,
 * or `-1` when `argv` doesn't run a script.
 * Only `run` as the first arg counts, the documented form.
 */
const to_run_path_index = (argv: Array<string>): number => {
	if (argv[2] !== RUN_TASK_NAME) return -1;
	for (let i = 3; i < argv.length; i++) {
		const arg = argv[i]!;
		if (arg === '--') return -1;
		if (!arg.startsWith('-')) return i;
	}
	return -1;
};

/**
 * Parses the task and its `args` from `process.argv`,
 * ignoring anything after any `--`.
 * `gro task <path>` consumes one more positional, the path.
 * `gro run [--help] <path> ...rest` refuses any other flag before the path
 * and appends the path and `rest` verbatim to the positionals,
 * unparsed and including any `--`, so the script gets exactly what follows its path.
 * @throws TaskError when a flag replaces the positional args, like `--_` or `--no-_`,
 *   or `gro run` has a flag other than `--help` before its path
 */
export const to_task_args = (argv = process.argv): CliTaskArgs => {
	const run_path_index = to_run_path_index(argv);
	const forwarded_index = argv.indexOf('--');
	const end =
		run_path_index !== -1 ? run_path_index : forwarded_index === -1 ? argv.length : forwarded_index;
	const raw_argv = argv.slice(2, end);
	const args = argv_parse(raw_argv);
	const positionals = to_positionals(args, raw_argv);
	const task_name = to_optional_string(positionals.shift()) ?? '';
	const task_path = task_name === TASK_VERB ? to_optional_string(positionals.shift()) : null;
	if (run_path_index !== -1) {
		if (Object.keys(args).some((key) => key !== '_' && !RUN_FLAGS.has(key))) {
			// the hint repeats the raw text, so a `--no-x` or a flag's value stays as written
			const flags = raw_argv.slice(1).filter((arg) => !/^--(no-)?help(=|$)/.test(arg));
			throw new TaskError(
				`\`gro ${RUN_TASK_NAME}\` takes no flags before the path, so flags for the script go after it: ` +
					`\`gro ${RUN_TASK_NAME} <path> ${flags.join(' ')}\``
			);
		}
		positionals.push(...argv.slice(run_path_index));
	}
	if (!positionals.length) delete (args as Args)._; // enable schema defaults
	return { task_name, task_path, args };
};

type Positional = string | number | boolean;

const to_optional_string = (v: Positional | undefined): string | null =>
	v === undefined ? null : String(v);

/**
 * Gets the positional args parsed from `argv`,
 * throwing a usage error when a flag like `--_` or `--no-_` replaced them.
 */
const to_positionals = (args: Args, argv: Array<string>): Array<Positional> => {
	const positionals: unknown = args._;
	if (Array.isArray(positionals)) return positionals;
	throw new TaskError(
		`\`_\` holds the positional args and can't be set by a flag like \`--_\` or \`--no-_\`: \`${argv.join(' ')}\``
	);
};

/**
 * Gets the array of raw string args starting with the first `--`, if any.
 * `gro run <path> ...rest` has none, as everything after the path is the script's.
 */
export const to_raw_rest_args = (argv = process.argv): Array<string> => {
	if (to_run_path_index(argv) !== -1) return [];
	const forwarded_index = argv.indexOf('--');
	return forwarded_index === -1 ? [] : argv.slice(forwarded_index);
};

/**
 * Splits `raw_rest_args` into its sections, each the verbatim args after a `--`
 * up to the next, skipping empty sections.
 */
const to_forwarded_sections = (raw_rest_args: Array<string>): Array<Array<string>> => {
	const sections: Array<Array<string>> = [];
	let section: Array<string> | undefined;
	for (const arg of raw_rest_args) {
		if (arg === '--') {
			if (section?.length) sections.push(section);
			section = [];
		} else if (section) {
			section.push(arg);
		}
	}
	if (section?.length) sections.push(section);
	return sections;
};

/**
 * The command that marks a forwarded section as args to a Gro task.
 */
const GRO_COMMAND = 'gro';

/**
 * Gets the verbatim args forwarded to the outside tool `command`, unparsed,
 * from the section after a `--` whose first arg is `command`, the last one when several are.
 * Given `gro check -- eslint --max-warnings -1 -- tsc --noEmit`,
 * `'eslint'` returns `['--max-warnings', '-1']`.
 * Gro tasks get parsed args with `to_forwarded_args` instead.
 * A literal `--` can't be forwarded, as it separates the sections.
 * @param command - the tool's CLI name, the section's first arg
 * @param raw_rest_args - the CLI args from the first `--` on, defaults to those of `process.argv`
 * @returns the args after `command`, empty when no section names it
 */
export const to_forwarded_argv = (
	command: string,
	raw_rest_args = to_raw_rest_args()
): Array<string> =>
	to_forwarded_sections(raw_rest_args)
		.findLast((section) => section[0] === command)
		?.slice(1) ?? [];

/**
 * Gets the verbatim args of the first section after a `--`
 * for a tool run by `task_name` without naming it, like `gro test -- --watch` for vitest,
 * only when `task_name` is the task the CLI invoked, so a nested task gets none.
 * A first section starting with `gro` (a Gro task's) or with `command` (an explicit section,
 * read by `to_forwarded_argv`) is none.
 * @param task_name - the asking task's name, a builtin name or a normalized task path
 * @param command - the tool's CLI name, whose explicit section isn't implicit
 * @param argv - the CLI args, defaults to `process.argv`
 * @returns the first section's args, empty when absent or not for `task_name`
 */
export const to_implicit_forwarded_argv = (
	task_name: string,
	command?: string,
	argv = process.argv
): Array<string> => {
	const raw_rest_args = to_raw_rest_args(argv);
	if (!raw_rest_args.length) return [];
	const cli = to_task_args(argv);
	const cli_task_name =
		cli.task_name === TASK_VERB
			? cli.task_path === null
				? null
				: to_task_path(cli.task_path)
			: cli.task_name;
	if (cli_task_name !== task_name) return [];
	// only the section right after the first `--`, empty when another `--` follows at once
	const end = raw_rest_args.indexOf('--', 1);
	const section = raw_rest_args.slice(1, end === -1 ? undefined : end);
	const first = section[0];
	if (first === GRO_COMMAND || (command !== undefined && first === command)) return [];
	return section;
};

/**
 * Whether `forwarded_argv` names the flag `name`, by a scan of flag names, not a parse:
 * `--name`, `--name=…`, `--no-name`, or `--no-name=…`,
 * and a one-character `name` also as a short flag starting `-name`, like `-o` or `-odist`
 * (not a later letter of a group like `-wo`).
 * Tasks use it to drop a flag of their own that the user's forwarded text overrides.
 * @param forwarded_argv - the verbatim forwarded args, from `to_forwarded_argv`
 * @param name - the flag name without dashes
 * @param ignore_case - match the long forms case-insensitively, for tools like `tsc`
 */
export const has_forwarded_flag = (
	forwarded_argv: Array<string>,
	name: string,
	ignore_case = false
): boolean => {
	const target = ignore_case ? name.toLowerCase() : name;
	return forwarded_argv.some((arg) => {
		if (arg.startsWith('--')) {
			const equals_index = arg.indexOf('=');
			let flag = equals_index === -1 ? arg.slice(2) : arg.slice(2, equals_index);
			if (ignore_case) flag = flag.toLowerCase();
			return flag === target || flag === 'no-' + target;
		}
		return name.length === 1 && arg.length > 1 && arg[0] === '-' && arg[1] === name;
	});
};

/**
 * Gets the value of the last flag `name` in `forwarded_argv`, by a scan, not a parse:
 * `--name value` or `--name=value`, and for a one-character `short`, `-short value` or `-shortvalue`,
 * not inside a group of short flags like `-wo dir`.
 * @param forwarded_argv - the verbatim forwarded args, from `to_forwarded_argv`
 * @param name - the long flag name without dashes
 * @param short - the one-character short flag name, if any
 * @returns the value, or `null` when the flag is absent or has no value
 */
export const to_forwarded_flag_value = (
	forwarded_argv: Array<string>,
	name: string,
	short?: string
): string | null => {
	let value: string | null = null;
	for (let i = 0; i < forwarded_argv.length; i++) {
		const arg = forwarded_argv[i]!;
		const next = forwarded_argv[i + 1];
		if (arg === '--' + name || (short !== undefined && arg === '-' + short)) {
			value = next ?? null;
			i++;
		} else if (arg.startsWith('--' + name + '=')) {
			value = arg.slice(name.length + 3);
		} else if (short !== undefined && arg.length > 2 && arg.startsWith('-' + short)) {
			value = arg.slice(2);
		}
	}
	return value;
};

/**
 * Gets the parsed args forwarded to the Gro task `command`,
 * like `gro test` given `gro check -- gro test --coverage`, or `{}` when none are.
 * Outside tools get verbatim args with `to_forwarded_argv` instead.
 * @param command - `gro` and the task name, from `to_task_command`
 * @param raw_rest_args - the CLI args from the first `--` on, defaults to those of `process.argv`
 * @throws TaskError when a `gro` section is malformed, see `to_forwarded_args_by_command`
 */
export const to_forwarded_args = (command: string, raw_rest_args?: Array<string>): Args =>
	to_forwarded_args_by_command(raw_rest_args)[command] ?? {};

/**
 * Merges `forwarded` args over `args`, moving each forwarded key after every key of `args`.
 * `args_parse` resolves a `no-x` pair given both ways to the later key,
 * so a forwarded `--color` overrides a direct `-C` (an alias of `no-color`),
 * where a spread keeps the overridden key at its original position.
 * Keys match by spelling, not by schema alias, so a forwarded alias (`-n`) doesn't displace
 * a direct canonical key (`--name`), which `args_parse` prefers.
 * @param args - the direct args, if any, not mutated
 * @param forwarded - the args forwarded to this command, which win over `args`
 * @returns a new object with the keys of `args` not in `forwarded`, then the keys of `forwarded`
 */
export const merge_forwarded_args = (args: Args | undefined, forwarded: Args): Args => {
	// `fromEntries` and spread define own keys, so an own `__proto__` arg stays an own key
	const unforwarded = args
		? Object.fromEntries(Object.entries(args).filter(([key]) => !Object.hasOwn(forwarded, key)))
		: {};
	return { ...unforwarded, ...forwarded };
};

/**
 * Parses each Gro task section of `raw_rest_args`, those after a `--` whose first arg is `gro`,
 * into args keyed by `gro` and the task name: `-- gro test --a` is keyed `gro test`,
 * and `-- gro task src/lib/x.task.ts --a` is keyed by `gro task` and the normalized path.
 * Other sections are left verbatim for outside tools, see `to_forwarded_argv`.
 * When a task has several sections, the last one wins.
 * @param raw_rest_args - the CLI args from the first `--` on, defaults to those of `process.argv`
 * @returns the forwarded args of each Gro task command
 * @throws TaskError when a `gro` section names no task, a `gro task` section no path,
 *   a section names a path without `gro task` or a task with the removed `gro/` prefix,
 *   or a flag replaces a section's positional args
 */
export const to_forwarded_args_by_command = (
	raw_rest_args = to_raw_rest_args()
): Record<string, Args | undefined> => {
	const forwarded_args_by_command: Record<string, Args> = {};
	for (const argv of to_forwarded_sections(raw_rest_args)) {
		if (argv[0] !== GRO_COMMAND) continue;
		const args = argv_parse(argv.slice(1));
		const positionals = to_positionals(args, argv);
		const task_name = to_optional_string(positionals.shift());
		if (!task_name) {
			throw new TaskError(
				`Malformed args following a \`--\`. Expected gro taskname: \`${argv.join(' ')}\``
			);
		}
		let command = GRO_COMMAND + ' ' + task_name;
		if (task_name === TASK_VERB) {
			const task_path = to_optional_string(positionals.shift());
			if (!task_path) {
				throw new TaskError(
					`Malformed args following a \`--\`. Expected a path after \`gro task\`: \`${argv.join(' ')}\``
				);
			}
			command += ' ' + to_task_path(task_path);
		} else if (is_removed_builtin_prefix(task_name)) {
			throw new TaskError(to_removed_builtin_prefix_message(task_name));
		} else if (is_task_path_input(task_name)) {
			throw new TaskError(to_task_path_without_verb_message(task_name));
		}
		if (!positionals.length) delete (args as Args)._;
		forwarded_args_by_command[command] = args;
	}
	return forwarded_args_by_command;
};

export const print_command_args = (serialized_args: Array<string>): string =>
	st('gray', '[') +
	st('magenta', 'running command') +
	st('gray', ']') +
	' ' +
	serialized_args.join(' ');
