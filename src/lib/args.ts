/**
 * The args host module: the arguments after the script, as an import rather than a global.
 *
 * `args` is a frozen array read when this module loads, what a script given to `gro run`
 * receives and what a bare `node script.ts` receives. In the CLI's own process they're
 * the CLI's args, the task name included, since a task's parsed args are its context's `task_args`.
 *
 * `args` is no handle but a real array, frozen, so its prototype is the unfrozen `Array.prototype`
 * and awaiting it adopts a polluted `then`.
 *
 * This is the Node implementation, which runs on the Node Gro. Its gaps against
 * the runtime's builtin, which it documents rather than the API weakening to match:
 *
 * - The global `process` stays, and any module can read `process.argv`.
 * - `args` under `node -e` and `node -p` reads the undocumented `process._eval`,
 *   as no documented API says the process has no script.
 *
 * @module
 */

import * as node_worker_threads from 'node:worker_threads';

const { isMainThread } = node_worker_threads;
const { freeze } = Object;

/**
 * The arguments after the script. `node -e` and `node -p` have no script, so the first
 * argument is at `argv[1]`; a script read from stdin as `-` keeps the name at `argv[1]`.
 * A worker always has something at `argv[1]`, its script or an eval marker, whatever `_eval` says.
 * `node -e code - a` reads as the stdin case, so its `-` is dropped, a known limit.
 */
const to_script_args = (): ReadonlyArray<string> => {
	const { argv, _eval } = globalThis.process as { argv: Array<string>; _eval?: string };
	const start = isMainThread && typeof _eval === 'string' && argv[1] !== '-' ? 1 : 2;
	const script_args: Array<string> = [];
	const length = argv.length;
	for (let i = start; i < length; i++) script_args[script_args.length] = argv[i]!;
	return freeze(script_args);
};

/**
 * The arguments after the script, frozen. In the CLI's own process these are the CLI's,
 * the task name included; a task reads its parsed args from its context.
 */
export const args: ReadonlyArray<string> = to_script_args();
