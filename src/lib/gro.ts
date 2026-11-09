#!/usr/bin/env -S node --experimental-import-meta-resolve

// @sync Node options to `gro_helpers.ts`

import { join } from 'node:path';

import { resolve_gro_module_path, spawn_with_loader } from './gro_helpers.ts';
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
	const spawned = await spawn_with_loader(node, loader_path, invoke_path, process.argv.slice(2));
	if (spawned.kind !== 'exited' || spawned.code !== 0) {
		process.exitCode = spawned.kind === 'exited' ? spawned.code : 1;
	}
} catch (error) {
	if (!is_spawn_error(error)) throw error;
	// no stack, as this is a known failure before any task
	const hint = is_spawn_error(error, 'not_found')
		? ' (Gro needs `node` on an absolute PATH entry)'
		: '';
	process.stderr.write(`${error.message}${hint}\n`);
	process.exitCode = 1;
}
