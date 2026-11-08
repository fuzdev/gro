import type { Logger } from '@fuzdev/fuz_util/log.ts';
import { UnreachableError } from '@fuzdev/fuz_util/error.ts';
import { styleText as st } from 'node:util';

import { NODE_MODULES_DIRNAME } from './constants.ts';
import type {
	Spawn,
	SpawnCallOptions,
	SpawnChild,
	SpawnProgram,
	SpawnProgramOptions,
	SpawnResult,
	SpawnStartOptions
} from './spawn.ts';
import { is_spawn_error, type SpawnError } from './spawn_error.ts';
import { TaskError } from './task.ts';

const LOCAL_BIN_PREFIX = `${NODE_MODULES_DIRNAME}/.bin/`;

/**
 * A program that couldn't be reached or started fails the task with that message,
 * a known failure rather than an unexpected error.
 */
const to_cli_task_error = (error: SpawnError): TaskError =>
	new TaskError(error.message, { cause: error });

const find_program = (
	spawn: Spawn,
	name: string,
	options: SpawnProgramOptions | undefined
): SpawnProgram | null => {
	try {
		return spawn.find(name, options);
	} catch (error) {
		if (is_spawn_error(error)) throw to_cli_task_error(error);
		throw error;
	}
};

/**
 * Finds the project's own install of the CLI `name` in `node_modules/.bin`.
 * @param spawn - resolves the program
 * @param name - the CLI's name
 * @param options - the program's environment, fixed when its handle is made
 * @returns the program, or `null` if the project doesn't have it
 * @throws `TaskError` - if the file isn't one the kernel will run or the name isn't allowed,
 * with the `SpawnError`'s message and the error as its `cause`
 */
export const find_local_cli = (
	spawn: Spawn,
	name: string,
	options?: SpawnProgramOptions
): SpawnProgram | null => find_program(spawn, LOCAL_BIN_PREFIX + name, options);

/**
 * Finds the CLI `name`, first in the project's `node_modules/.bin` and then on `PATH`.
 * Only this lookup reads `node_modules/.bin`, so a dependency's `.bin/git`
 * can't shadow a bare `spawn.program('git')`.
 * @param spawn - resolves the program
 * @param name - the CLI's name
 * @param options - the program's environment, fixed when its handle is made
 * @returns the program, or `null` if not found locally or on `PATH`
 * @throws `TaskError` - if the file isn't one the kernel will run or the name isn't allowed,
 * like an absolute one, with the `SpawnError`'s message and the error as its `cause`
 */
export const find_cli = (
	spawn: Spawn,
	name: string,
	options?: SpawnProgramOptions
): SpawnProgram | null =>
	find_local_cli(spawn, name, options) ?? find_program(spawn, name, options);

/**
 * A program's name for a log line, a local bin by its bare name.
 */
const to_cli_name = ({ name }: SpawnProgram): string =>
	name.startsWith(LOCAL_BIN_PREFIX) ? name.slice(LOCAL_BIN_PREFIX.length) : name;

const log_cli = (program: SpawnProgram, args: ReadonlyArray<string>, log?: Logger): void => {
	if (log) log.info(print_command_args([to_cli_name(program), ...args]));
};

/**
 * Runs a CLI with stdio inherited, resolving when it ends.
 * @param program - the CLI, usually from `find_cli`
 * @param args - the arguments after `argv[0]`
 * @param log - logs the command when present
 * @param options - the call's options
 * @returns how it ended, a non-zero exit being a result
 * @throws `TaskError` - if it couldn't start, with the `SpawnError`'s message and the error
 * as its `cause`
 */
export const spawn_cli = async (
	program: SpawnProgram,
	args: ReadonlyArray<string> = [],
	log?: Logger,
	options?: SpawnCallOptions
): Promise<SpawnResult> => {
	log_cli(program, args, log);
	try {
		return await program.run(args, options);
	} catch (error) {
		if (is_spawn_error(error)) throw to_cli_task_error(error);
		throw error;
	}
};

/**
 * Starts a long-lived CLI, stdio inherited unless `options` says otherwise.
 * @param program - the CLI, usually from `find_cli`
 * @param args - the arguments after `argv[0]`
 * @param log - logs the command when present
 * @param options - the call's options
 * @returns the running child
 * @throws `TaskError` - if it couldn't start, with the `SpawnError`'s message and the error
 * as its `cause`
 */
export const spawn_cli_process = async (
	program: SpawnProgram,
	args: ReadonlyArray<string> = [],
	log?: Logger,
	options?: SpawnStartOptions
): Promise<SpawnChild> => {
	log_cli(program, args, log);
	try {
		return await program.start(args, options);
	} catch (error) {
		if (is_spawn_error(error)) throw to_cli_task_error(error);
		throw error;
	}
};

/**
 * Whether a program finished by exiting 0.
 */
export const spawn_result_is_ok = (result: SpawnResult): boolean =>
	result.kind === 'exited' && result.code === 0;

/**
 * Formats how a program ended, for an error message.
 */
export const to_spawn_result_message = (result: SpawnResult): string => {
	switch (result.kind) {
		case 'exited':
			return `code ${result.code}`;
		case 'signaled':
			return `signal ${result.signal}`;
		case 'timed_out':
			return 'timed out';
		case 'aborted':
			return 'aborted';
		default:
			throw new UnreachableError(result);
	}
};

/**
 * Formats a command and its args for the log line that announces running it.
 * @param serialized_args - the command followed by its args
 */
export const print_command_args = (serialized_args: Array<string>): string =>
	st('gray', '[') +
	st('magenta', 'running command') +
	st('gray', ']') +
	' ' +
	serialized_args.join(' ');
