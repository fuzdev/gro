import type * as esbuild from 'esbuild';
import { readFile } from 'node:fs/promises';
import { dirname, extname } from 'node:path';

import { PATH_SPECIFIER_MATCHER, resolve_specifier, strip_raw_query } from './resolve_specifier.ts';
import { EVERYTHING_MATCHER } from './constants.ts';
import { replace_extension } from './paths.ts';

/**
 * Matches a subpath import specifier, like `#lib/x.ts`, resolved through package.json `imports`.
 */
const SUBPATH_IMPORT_MATCHER = /^#/;

/**
 * Marks the `build.resolve` call that resolves a subpath import,
 * so this plugin's own `onResolve` passes it to esbuild instead of recursing.
 */
const SUBPATH_IMPORT_RESOLVING = 'gro_subpath_import_resolving';

/**
 * Resolves relative and absolute imports, the specifiers `is_path_specifier` matches,
 * to exactly the file they name,
 * loading `.ts` as TypeScript. Extensions are mandatory and `.js` never maps to `.ts`,
 * so a specifier that doesn't name an existing file fails the build.
 * Subpath imports like `#lib/x.ts` resolve through package.json `imports` as esbuild does,
 * but esbuild's own rewrite of a missing `.js` to its `.ts` sibling, like `#lib/x.js` to `x.ts`,
 * fails the build with the fix, as Node and the loader never make it;
 * a mapping that names another extension on purpose, like `"#x.js": "./x.ts"`, resolves.
 * A known limitation: a deliberate mapping overlapped by a catch-all,
 * like `"#lib/*.js": "./src/lib/*.ts"` beside `"#lib/*": "./src/lib/*"`, is refused,
 * since its `.ts` spelling resolves to the same file, as with esbuild's rewrite;
 * telling them apart needs Node's resolution, which takes a flag not every caller has.
 * Imports from inside `node_modules` use esbuild's default resolution.
 */
export const esbuild_plugin_sveltekit_local_imports = (): esbuild.Plugin => ({
	name: 'sveltekit_local_imports',
	setup: (build) => {
		build.onResolve({ filter: PATH_SPECIFIER_MATCHER }, async (args) => {
			const { path, importer } = args;
			if (!importer) return { path };
			if (importer.includes('/node_modules/')) return undefined;
			const { path_id, namespace } = await resolve_specifier(path, importer);
			return { path: path_id, namespace }; // `namespace` may be `undefined`, but esbuild needs the absolute path for json etc
		});
		build.onResolve({ filter: SUBPATH_IMPORT_MATCHER }, async (args) => {
			const { path, importer, kind, resolveDir, namespace, pluginData } = args;
			if (pluginData?.[SUBPATH_IMPORT_RESOLVING] || importer.includes('/node_modules/')) {
				return undefined;
			}
			const resolve_options: esbuild.ResolveOptions = {
				kind,
				importer,
				resolveDir,
				namespace,
				pluginData: { [SUBPATH_IMPORT_RESOLVING]: true }
			};
			const resolved = await build.resolve(path, resolve_options);
			if (resolved.errors.length || resolved.external) return resolved;
			// esbuild maps a missing `.js` to its `.ts` sibling, where Node never does;
			// a mapping that targets another extension on purpose, like `"#x.js": "./x.ts"`,
			// is told apart because the specifier with the resolved extension doesn't resolve to the same file
			const bare_path = strip_raw_query(path);
			const ext = extname(bare_path);
			const resolved_ext = extname(resolved.path);
			if (!ext || resolved_ext === ext) return resolved;
			const corrected = replace_extension(bare_path, resolved_ext);
			const corrected_resolved = await build.resolve(corrected, resolve_options);
			if (corrected_resolved.errors.length || corrected_resolved.path !== resolved.path) {
				return resolved;
			}
			return {
				errors: [
					{
						text:
							`cannot resolve '${path}' from ${importer}: no such file, ` +
							`but '${corrected}${path.slice(bare_path.length)}' exists; ` +
							`write the real extension (${ext} never maps to ${resolved_ext})`
					}
				]
			};
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
