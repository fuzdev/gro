import type { SpawnProgram, SpawnResult } from './spawn.ts';
import { to_file_url } from './internal/posix_path.ts';

/*

This module is intended to have minimal dependencies to avoid over-imports in the CLI,
and imports no `node:` builtin, so the plugins that run a file with the loader load without Node.

*/

/**
 * Runs a file using the Gro loader.
 *
 * Uses conditional exports to correctly set up `esm-env` as development by default,
 * so if you want production set `NODE_ENV=production`.
 *
 * @param node - the `node` program, whose environment the root decides
 * @param loader_path - absolute path to the loader
 * @param invoke_path - path to file to spawn with `node`
 * @param argv - the args after `invoke_path`
 * @param node_env - the host's `NODE_ENV`, read by the root
 * @throws `TypeError` - when `loader_path` isn't absolute
 * @see https://nodejs.org/api/packages.html#conditional-exports
 */
export const spawn_with_loader = (
	node: SpawnProgram,
	loader_path: string,
	invoke_path: string,
	argv: ReadonlyArray<string>,
	node_env: string | undefined
): Promise<SpawnResult> =>
	node.run(spawn_with_loader_args(loader_path, invoke_path, argv, node_env));

/**
 * The Node args that run `invoke_path` with Gro's loader registered.
 * @param loader_path - absolute path to the loader
 * @param node_env - the host's `NODE_ENV`, read by the root
 * @throws `TypeError` - when `loader_path` isn't absolute
 */
export const spawn_with_loader_args = (
	loader_path: string,
	invoke_path: string,
	argv: ReadonlyArray<string>,
	node_env: string | undefined
): Array<string> => {
	const args = [
		'--import',
		// does what `register.ts` does without importing another file; the source is encoded,
		// and the loader registered by its file URL, so URL or JS string syntax in its path is data,
		// though Node's resolver refuses a `\` in it, which the file URL encodes
		to_register_loader_url(loader_path),
		// @sync Node options to `gro.ts`
		'--experimental-import-meta-resolve' // for `import.meta.resolve`
	];
	// In almost all cases we want the exports condition to be `"development"`. Needed for `esm-env`.
	if (node_env !== 'production') {
		args.push('-C', 'development'); // same as `--conditions`
	}
	args.push(invoke_path, ...argv);
	return args;
};

// the `data:` URL of a module that registers the loader at `loader_path`, for `--import`
const to_register_loader_url = (loader_path: string): string =>
	'data:text/javascript,' +
	encodeURIComponent(
		`import {register} from "node:module"; register(${JSON.stringify(to_file_url(loader_path))});`
	);
