#!/usr/bin/env -S node --experimental-import-meta-resolve

// @sync Node options to `gro_helpers.ts`

import { join } from 'node:path';

import { resolve_gro_module_path, spawn_with_loader, to_signal_exit_code } from './gro_helpers.ts';
import { args } from './args.ts';
import { env } from './env.ts';
import { set_exit_code } from './exit.ts';
import { stderr } from './stderr.ts';
import { spawn } from './spawn.ts';
import { is_spawn_error } from './spawn_error.ts';

/*

This file is a loader for the Gro CLI.
Its only purpose is to import the `invoke.js` script in the correct directory.
By using `resolve_gro_module_path` it lets the global Gro CLI defer
to a local installation of Gro if one is available,
and it also provides special handling for the case
where we're running Gro inside Gro's own repo for development.

*/

const invoke_path = resolve_gro_module_path('invoke.js');

const loader_path = join(invoke_path, '../loader.js');

try {
	// the CLI forwards the whole environment to the process that runs the task
	const node = spawn.program('node', { pass_env: 'all' });
	const spawned = await spawn_with_loader(
		node,
		loader_path,
		invoke_path,
		args,
		env.get('NODE_ENV')
	);
	if (spawned.kind !== 'exited' || spawned.code !== 0) {
		// a signal exits like a shell, `128` plus its number
		set_exit_code(
			spawned.kind === 'exited'
				? spawned.code
				: spawned.kind === 'signaled'
					? to_signal_exit_code(spawned.signal)
					: 1
		);
	}
} catch (error) {
	if (!is_spawn_error(error)) throw error;
	// no stack, as this is a known failure before any task
	const hint = is_spawn_error(error, 'not_found')
		? ' (Gro needs `node` on an absolute PATH entry)'
		: '';
	await stderr.write(`${error.message}${hint}\n`);
	set_exit_code(1);
}
