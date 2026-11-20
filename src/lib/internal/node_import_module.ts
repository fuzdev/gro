/**
 * Node's `ImportModule`, how the Node Gro's composition roots load a project's files.
 *
 * Node-only. The runtime's entry hands Gro its own.
 *
 * @module
 */

import { pathToFileURL } from 'node:url';

import type { ImportModule } from '../modules.ts';

/**
 * Imports the module at `id` by its file URL, so a `#`, `?`, or `%` in the path is escaped,
 * not read as URL syntax.
 */
export const import_module_node: ImportModule = (id) => import(pathToFileURL(id).href);
