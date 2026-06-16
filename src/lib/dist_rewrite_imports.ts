import {fs_search} from '@fuzdev/fuz_util/fs.js';
import {map_concurrent} from '@fuzdev/fuz_util/async.js';
import type {Logger} from '@fuzdev/fuz_util/log.js';
import {readFile, writeFile} from 'node:fs/promises';

import {SVELTE_SCRIPT_MATCHER, SVELTEKIT_DIST_DIRNAME} from './constants.ts';

/*

Post-`svelte-package` pass that rewrites relative `.ts` import specifiers to `.js`
in the published `dist` output.

The ecosystem writes import specifiers with the real source extension
(`./foo.ts`, `./bar.svelte.ts`) and relies on emit-only rewrites to make `dist`
resolve for external consumers. `tsc`'s `rewriteRelativeImportExtensions` handles
the `.ts`/`.svelte.ts` → `dist/*.js` emit, but two outputs are left untouched:

- `.svelte` files, which `svelte-package` ships verbatim (source `<script>` and
  its relative specifiers intact), and
- `.d.ts` / `*.svelte.d.ts` declaration files, whose specifiers `svelte-package`'s
  declaration emit leaves as `.ts`.

This pass closes both gaps so a flagless external consumer (no
`allowImportingTsExtensions`) resolves the package's types and runtime.

Only **relative** specifiers (`./`, `../`) are rewritten. Bare `@fuzdev/…ts`
specifiers are left alone — the package `exports` `.js`/`.ts` mirror resolves them
in both source and dist. `.svelte` component imports and specifiers already ending
in `.js` are likewise untouched.

This is intentionally parse-light; the long-term home for the rewrite is tsv.

*/

/**
 * Matches a relative import/export specifier ending in `.ts` (including the
 * `.svelte.ts` double extension), capturing the introducer, the opening quote,
 * and the path body so the trailing `.ts` can be swapped for `.js`.
 *
 * Anchored on a module-specifier introducer (`from`, `import`, `require`) so it
 * doesn't touch incidental relative-looking string literals, on a `./`/`../`
 * prefix so bare specifiers like `@fuzdev/fuz_util/foo.ts` are left untouched, and
 * on a `(?<!\.)` lookbehind so member calls like `arr.from('./x.ts')` are skipped.
 */
const RELATIVE_TS_IMPORT_MATCHER =
	/(?<!\.)((?:\bfrom|\bimport|\brequire)\b\s*\(?\s*)(['"])(\.\.?\/[^'"\n]*?)\.ts\2/g;

/**
 * Rewrites relative `.ts` (and `.svelte.ts`) import specifiers to `.js` (and
 * `.svelte.js`) across a chunk of TypeScript source — a whole `.d.ts` file or a
 * `.svelte` `<script>` block.
 */
export const rewrite_relative_ts_imports = (content: string): string =>
	content.replace(
		RELATIVE_TS_IMPORT_MATCHER,
		(_match, intro: string, quote: string, body: string) => `${intro}${quote}${body}.js${quote}`,
	);

/**
 * Rewrites relative `.ts` import specifiers to `.js` inside each `<script>` block
 * of a `.svelte` file, leaving template markup untouched.
 */
export const rewrite_svelte_ts_imports = (content: string): string =>
	content.replace(SVELTE_SCRIPT_MATCHER, (full: string, inner: string) => {
		const rewritten = rewrite_relative_ts_imports(inner);
		if (rewritten === inner) return full;
		// replace via a function so `$`-sequences in the script (`$$props`, `$:`,
		// `$&`, …) aren't interpreted as `String.prototype.replace` substitution patterns
		return full.replace(inner, () => rewritten);
	});

export interface RewriteDistImportsResult {
	/** Number of files scanned (`.d.ts` and `.svelte`). */
	scanned: number;
	/** Number of scanned files whose contents changed. */
	rewritten: number;
}

/** Bounds open file descriptors while rewriting the dist tree. */
const DIST_REWRITE_CONCURRENCY = 16;

/**
 * Walks `dist_dir` and rewrites relative `.ts` import specifiers to `.js` in every
 * `.d.ts` declaration file (whole-file) and `.svelte` file (`<script>` blocks
 * only). `.js` files are skipped — `tsc`'s `rewriteRelativeImportExtensions`
 * already rewrites those — as are source maps.
 *
 * @param dist_dir - the packaged output directory; defaults to `dist`
 * @returns counts of files scanned and rewritten
 */
export const rewrite_dist_imports = async (
	dist_dir: string = SVELTEKIT_DIST_DIRNAME,
	log?: Logger,
): Promise<RewriteDistImportsResult> => {
	const found = await fs_search(dist_dir, {
		file_filter: (id) => id.endsWith('.d.ts') || id.endsWith('.svelte'),
	});

	const changed = await map_concurrent(found, DIST_REWRITE_CONCURRENCY, async ({id, path}) => {
		const content = await readFile(id, 'utf8');
		const next = id.endsWith('.svelte')
			? rewrite_svelte_ts_imports(content)
			: rewrite_relative_ts_imports(content);
		if (next === content) return false;
		await writeFile(id, next);
		log?.debug(`rewrote relative .ts import specifiers to .js in ${path}`);
		return true;
	});
	const rewritten = changed.filter(Boolean).length;

	if (found.length) {
		log?.info(
			`rewrote relative .ts→.js import specifiers in ${rewritten}/${found.length} dist files`,
		);
	}

	return {scanned: found.length, rewritten};
};
