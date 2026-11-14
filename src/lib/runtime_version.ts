/**
 * The runtime_version host module: the name and version of the runtime running the process,
 * as an import rather than a global.
 *
 * This is the Node implementation, which runs on the Node Gro. Its gap against the runtime's
 * builtin, which it documents rather than the API weakening to match: the global `process`
 * stays, and any module can read `process.version`.
 *
 * @module
 */

/**
 * The runtime's name and version, opaque, like `node/v24.14.0` here and `fgro/0.1.0` on the
 * runtime, for display and for keying caches by the runtime that made them.
 * Compare it for equality only; a program that branches on it is one this module can't serve.
 */
export const runtime_version: string = `node/${globalThis.process.version}`;
