import type {SpawnOptions} from 'node:child_process';
import {
	spawn,
	spawn_out,
	spawn_process,
	type SpawnResult,
	type SpawnedProcess,
} from '@fuzdev/fuz_util/process.js';
import {join} from 'node:path';
import {fs_exists} from '@fuzdev/fuz_util/fs.js';
import {fileURLToPath, type URL} from 'node:url';
import type {Logger} from '@fuzdev/fuz_util/log.js';
import type {PathId} from '@fuzdev/fuz_util/path.js';

import {NODE_MODULES_DIRNAME} from './constants.ts';
import {print_command_args} from './args.ts';

// TODO maybe upstream to fuz_util?

export type Cli =
	| {kind: 'local'; name: string; id: PathId}
	| {kind: 'global'; name: string; id: PathId};

/**
 * Searches the filesystem for the CLI `name`, first local to the cwd and then globally.
 * @returns `null` if not found locally or globally
 */
export const find_cli = async (
	name: string,
	cwd: string | URL = process.cwd(),
	options?: SpawnOptions,
): Promise<Cli | null> => {
	const final_cwd = typeof cwd === 'string' ? cwd : fileURLToPath(cwd);
	const local_id = join(final_cwd, NODE_MODULES_DIRNAME, `.bin/${name}`);
	if (await fs_exists(local_id)) {
		return {name, id: local_id, kind: 'local'};
	}
	const {stdout} = await spawn_out('sh', ['-c', 'command -v "$1"', 'sh', name], options);
	const global_id = stdout?.trim();
	if (!global_id) return null;
	return {name, id: global_id, kind: 'global'};
};

/**
 * Spawns a CLI if available using fuz_util's `spawn`.
 * If a string is provided for `name_or_cli`, it checks first local to the cwd and then globally.
 * @returns `undefined` if no CLI is found, or the spawn result
 */
export const spawn_cli = async (
	name_or_cli: string | Cli,
	args: Array<string> = [],
	log?: Logger,
	options?: SpawnOptions,
): Promise<SpawnResult | undefined> => {
	const cli = await resolve_cli(name_or_cli, args, options?.cwd, log, options);
	if (!cli) return;
	return spawn(cli.id, args, options);
};

/**
 * Spawns a CLI if available using fuz_util's `spawn_process`.
 * If a string is provided for `name_or_cli`, it checks first local to the cwd and then globally.
 * @returns `undefined` if no CLI is found, or the spawn result
 */
export const spawn_cli_process = async (
	name_or_cli: string | Cli,
	args: Array<string> = [],
	log?: Logger,
	options?: SpawnOptions,
): Promise<SpawnedProcess | undefined> => {
	const cli = await resolve_cli(name_or_cli, args, options?.cwd, log, options);
	if (!cli) return;
	return spawn_process(cli.id, args, options);
};

export const resolve_cli = async (
	name_or_cli: string | Cli,
	args: Array<string> = [],
	cwd: string | URL | undefined,
	log?: Logger,
	options?: SpawnOptions,
): Promise<Cli | undefined> => {
	let final_cli;
	if (typeof name_or_cli === 'string') {
		const found = await find_cli(name_or_cli, cwd, options);
		if (!found) return;
		final_cli = found;
	} else {
		final_cli = name_or_cli;
	}
	if (log) {
		log.info(print_command_args([final_cli.name].concat(args)));
	}
	return final_cli;
};

export const to_cli_name = (cli: string | Cli): string =>
	typeof cli === 'string' ? cli : cli.name;
