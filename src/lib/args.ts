import { styleText as st } from 'node:util';
import { argv_parse, type Args } from '@fuzdev/fuz_util/args.ts';

import { TaskError, to_task_path } from './task.ts';

/**
 * The CLI verb that runs or lists project tasks by path, `gro task [path]`.
 */
export const TASK_VERB = 'task';

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
 * Parses the task and its `args` from `process.argv`,
 * ignoring anything after any `--`.
 * `gro task <path>` consumes one more positional, the path.
 * @throws TaskError when a flag replaces the positional args, like `--_` or `--no-_`
 */
export const to_task_args = (argv = process.argv): CliTaskArgs => {
	const forwarded_index = argv.indexOf('--');
	const raw_argv = forwarded_index === -1 ? argv.slice(2) : argv.slice(2, forwarded_index);
	const args = argv_parse(raw_argv);
	const positionals = to_positionals(args, raw_argv);
	const task_name = to_optional_string(positionals.shift()) ?? '';
	const task_path = task_name === TASK_VERB ? to_optional_string(positionals.shift()) : null;
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
 */
export const to_raw_rest_args = (argv = process.argv): Array<string> => {
	const forwarded_index = argv.indexOf('--');
	return forwarded_index === -1 ? [] : argv.slice(forwarded_index);
};

/**
 * Parses `process.argv` for the specified `command`, so given
 * `gro taskname arg1 --arg2 -- eslint eslintarg1 --eslintarg2 -- tsc --tscarg1 --tscarg2`
 * the `command` `'eslint'` returns `eslintarg1 --eslintarg2`
 * and `'tsc'` returns `--tscarg1` and `--tscarg2`.
 */
export const to_forwarded_args = (
	command: string,
	raw_rest_args?: Array<string>,
	cache = to_forwarded_args_by_command(raw_rest_args)
): Args => cache[command] ?? {};

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

export const to_forwarded_args_by_command = (
	raw_rest_args = to_raw_rest_args()
): Record<string, Args | undefined> => {
	// Parse each segment of `argv` separated by `--`.
	const argvs: Array<Array<string>> = [];
	let arr: Array<string> | undefined;
	for (const arg of raw_rest_args) {
		if (arg === '--') {
			if (arr?.length) argvs.push(arr);
			arr = [];
		} else if (!arr) {
			continue;
		} else if (arg) {
			arr.push(arg);
		}
	}
	if (arr?.length) argvs.push(arr);
	// Add each segment of parsed `argv` keyed by the first rest arg,
	// which is assumed to be the CLI command that gets forwarded the args.
	const forwarded_args_by_command: Record<string, Args> = {};
	for (const argv of argvs) {
		const args = argv_parse(argv);
		const positionals = to_positionals(args, argv);
		let command = positionals.shift();
		if (!command) {
			// Skip sections without a command name - these are handled by `to_implicit_forwarded_args`.
			// This allows `gro run script.ts -- --help` to pass `--help` to the script.
			continue;
		}
		// Gro commands get combined with their task name,
		// and `gro task` with its path normalized like `invoke_task` keys it.
		if (command === 'gro') {
			const task_name = positionals.shift();
			if (!task_name) {
				throw new TaskError(
					`Malformed args following a \`--\`. Expected gro taskname: \`${argv.join(' ')}\``
				);
			}
			command += ' ' + task_name;
			if (task_name === TASK_VERB) {
				const task_path = positionals.shift();
				if (!task_path) {
					throw new TaskError(
						`Malformed args following a \`--\`. Expected a path after \`gro task\`: \`${argv.join(' ')}\``
					);
				}
				command += ' ' + to_task_path(String(task_path));
			}
		}
		if (!positionals.length) delete (args as Args)._;
		forwarded_args_by_command[String(command)] = args;
	}
	return forwarded_args_by_command;
};

/**
 * Gets all args after the first `--` without assuming a command name.
 * This is useful for tasks that want to forward args directly to a tool
 * without requiring users to specify the tool name explicitly.
 * Optionally strips a specific command name if present for backward compatibility.
 * @example
 * ```ts
 * // `gro test -- --watch` → {watch: true}
 * // `gro test -- foo.test.ts` → {_: ['foo.test.ts']}
 * // `gro test -- vitest --watch` with command_to_strip='vitest' → {watch: true}
 * to_implicit_forwarded_args('vitest')
 * ```
 */
export const to_implicit_forwarded_args = (
	command_to_strip?: string,
	raw_rest_args = to_raw_rest_args()
): Args => {
	const start = raw_rest_args.indexOf('--');
	if (start === -1) return {};

	let argv = raw_rest_args.slice(start + 1);

	if (command_to_strip && argv[0] === command_to_strip) {
		argv = argv.slice(1);
	}

	const args = argv_parse(argv);
	if (!to_positionals(args, argv).length) delete (args as Args)._;
	return args;
};

export const print_command_args = (serialized_args: Array<string>): string =>
	st('gray', '[') +
	st('magenta', 'running command') +
	st('gray', ']') +
	' ' +
	serialized_args.join(' ');
