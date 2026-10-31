import type * as esbuild from 'esbuild';
import { readFile } from 'node:fs/promises';
import { dirname } from 'node:path';

import { resolve_specifier } from './resolve_specifier.ts';
import { EVERYTHING_MATCHER } from './constants.ts';

/**
 * Resolves relative and absolute imports to exactly the file they name,
 * loading `.ts` as TypeScript. Extensions are mandatory and `.js` never maps to `.ts`,
 * so a specifier that doesn't name an existing file fails the build.
 * Imports from inside `node_modules` use esbuild's default resolution.
 */
export const esbuild_plugin_sveltekit_local_imports = (): esbuild.Plugin => ({
	name: 'sveltekit_local_imports',
	setup: (build) => {
		build.onResolve({ filter: /^(\/|\.)/ }, async (args) => {
			const { path, importer } = args;
			if (!importer) return { path };
			if (importer.includes('/node_modules/')) return undefined;
			const { path_id, namespace } = await resolve_specifier(path, importer);
			return { path: path_id, namespace }; // `namespace` may be `undefined`, but esbuild needs the absolute path for json etc
		});
		build.onLoad(
			{ filter: EVERYTHING_MATCHER, namespace: 'sveltekit_local_imports_ts' },
			async ({ path }) => ({
				contents: await readFile(path),
				loader: 'ts',
				resolveDir: dirname(path)
			})
		);
		build.onLoad(
			{ filter: EVERYTHING_MATCHER, namespace: 'sveltekit_local_imports_js' },
			async ({ path }) => ({
				contents: await readFile(path),
				resolveDir: dirname(path)
			})
		);
	}
});
