/**
 * The pid host module: the id of the running process, as an import rather than a global.
 *
 * This is the Node implementation, which runs on the Node Gro. Its gap against the runtime's
 * builtin, which it documents rather than the API weakening to match: the global `process`
 * stays, and any module can read `process.pid`.
 *
 * @module
 */

/**
 * The process's id, for display and for naming what only this process writes, like a temporary
 * or lock file. Nothing in Gro's host modules takes a pid, so holding one reaches no process.
 */
export const pid: number = globalThis.process.pid;
