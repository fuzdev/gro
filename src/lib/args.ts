import { styleText as st } from 'node:util';
import { argv_parse, type Args } from '@fuzdev/fuz_util/args.ts';

/**
 * Parses `task_name` and `args` from `process.argv`,
 * ignoring anything after any `--`.
 */
export const to_task_args = (argv = process.argv): { task_name: string; args: Args } => {
	const forwarded_index = argv.indexOf('--');
	const args = argv_parse(forwarded_index === -1 ? argv.slice(2) : argv.slice(2, forwarded_index));
	const task_name = args._.shift() ?? '';
	if (!args._.length) delete (args as Args)._; // enable schema defaults
	return { task_name, args };
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
		let command = args._.shift();
		if (!command) {
			// Skip sections without a command name - these are handled by `to_implicit_forwarded_args`.
			// This allows `gro run script.ts -- --help` to pass `--help` to the script.
			continue;
		}
		// Gro commands get combined with their task name.
		if (command === 'gro') {
			if (!args._.length) {
				throw Error(
					`Malformed args following a \`--\`. Expected gro taskname: \`${argv.join(' ')}\``
				);
			}
			command += ' ' + args._.shift();
		}
		if (!args._.length) delete (args as Args)._;
		forwarded_args_by_command[command] = args;
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
	if (!args._.length) delete (args as Args)._;
	return args;
};

export const print_command_args = (serialized_args: Array<string>): string =>
	st('gray', '[') +
	st('magenta', 'running command') +
	st('gray', ']') +
	' ' +
	serialized_args.join(' ');
