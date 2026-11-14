/**
 * The cwd host module: the working directory of the running process, as an import rather than
 * a global.
 *
 * The directory is read and checked once, when this module loads, as for `fs.project`, so
 * importing this module throws in a removed or non-UTF-8 working directory, as `fs` does.
 * `cwd` is a frozen function taking no receiver.
 *
 * This is the Node implementation, which runs on the Node Gro. Its gap against the runtime's
 * builtin, which it documents rather than the API weakening to match: the global `process`
 * stays, and any module can call `process.cwd` or `process.chdir`.
 *
 * @module
 */

import { to_checked_cwd } from './internal/cwd.ts';

const checked_cwd = to_checked_cwd();

/**
 * The working directory when the module loaded, checked as for `fs.project`.
 */
export const cwd = (): string => checked_cwd;

Object.freeze(cwd);
