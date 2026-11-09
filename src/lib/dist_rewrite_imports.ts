import { map_concurrent } from '@fuzdev/fuz_util/async.ts';
import type { Logger } from '@fuzdev/fuz_util/log.ts';
import { parse_imports_svelte, parse_imports_typescript } from '@fuzdev/tsv';
import { resolve } from 'node:path';

import { SVELTE_SCRIPT_MATCHER, SVELTEKIT_DIST_DIRNAME } from './constants.ts';
import { fs_search } from './fs_search.ts';
import type { FsPaths } from './fs_paths.ts';

/*

Post-`svelte-package` pass that rewrites relative `.ts` import specifiers to `.js`
in the published `dist` output.

The ecosystem writes import specifiers with the real source extension
(`./foo.ts`, `./bar.svelte.ts`) and relies on an emit-only rewrite to make `dist`
resolve for external consumers. This pass owns that rewrite end-to-end, across
every output `svelte-package` produces:

- `.js` runtime files (including `.svelte.js` compiled from `.svelte.ts`),
- `.svelte` files, which `svelte-package` ships verbatim (source `<script>` and
  its relative specifiers intact), and
- `.d.ts` / `*.svelte.d.ts` declaration files.

This closes the gap so a flagless external consumer (no
`allowImportingTsExtensions`) resolves the package's types and runtime.

It deliberately does **not** rely on `tsc`'s `rewriteRelativeImportExtensions`:
that flag rewrites only relative specifiers, so it can't carry a `#lib`/`#routes`
subpath import — a non-relative `.ts` specifier is a hard error under it,
which would force dissolving those imports into relative paths. With the rewrite
owned here, source keeps its subpath imports and the flag stays off. The pass is
idempotent, so it's a safe no-op on any `.js` `tsc` did happen to rewrite.

Only **relative** specifiers (`./`, `../`) are rewritten. Bare `@fuzdev/…ts`
specifiers are left alone — the package `exports` `.js`/`.ts` mirror resolves them
in both source and dist. Subpath imports (`#lib`, `#routes`) live only in
non-published app/test code, never in `dist`. `.svelte` component imports and
specifiers already ending in `.js` are likewise untouched.

`.js` and `.svelte` output is rewritten at the specifier spans tsv's import parse
reports, so a specifier-shaped string inside a template literal or comment (a
package's code-generation templates) stays as written. A file tsv can't parse falls
back to a pattern match over its text, with a warning, so it still ships resolvable.

`.d.ts` output stays on the pattern because tsv doesn't report the spans of
type-position `import('./y.ts')`. A declaration file has no code bodies, so all the
pattern can misread there is comments and string-typed declarations, which nothing
resolves, though a string-typed declaration can then disagree with its `.js` value,
which keeps `.ts`. The same parse reports no type-position `import()` in a `.svelte`
script, so those stay as written, erased at compile time. The long-term home for the
rewrite is tsv.

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
 * It sees text, not syntax, so it also matches inside strings and comments.
 */
const RELATIVE_TS_IMPORT_MATCHER =
	/(?<!\.)((?:\bfrom|\bimport|\brequire)\b\s*\(?\s*)(['"])(\.\.?\/[^'"\n]*?)\.ts\2/g;

/** Matches the text of a relative specifier ending in `.ts`, quotes excluded. */
const RELATIVE_TS_SPECIFIER_MATCHER = /^\.\.?\/[^'"\n]*\.ts$/;

/**
 * Rewrites relative `.ts` (and `.svelte.ts`) import specifiers to `.js` (and
 * `.svelte.js`) by pattern across a chunk of TypeScript source, including any
 * specifier-shaped text inside its strings and comments. For a dist file, use
 * `rewrite_dist_file_imports`.
 */
export const rewrite_relative_ts_imports = (content: string): string =>
	content.replace(
		RELATIVE_TS_IMPORT_MATCHER,
		(_match, intro: string, quote: string, body: string) => `${intro}${quote}${body}.js${quote}`
	);

/**
 * Rewrites relative `.ts` import specifiers to `.js` by pattern inside each
 * `<script>` block of a `.svelte` file, leaving template markup untouched.
 */
const rewrite_svelte_ts_imports = (content: string): string =>
	content.replace(SVELTE_SCRIPT_MATCHER, (full: string, inner: string) => {
		const rewritten = rewrite_relative_ts_imports(inner);
		if (rewritten === inner) return full;
		// replace via a function so `$`-sequences in the script (`$$props`, `$:`,
		// `$&`, …) aren't interpreted as `String.prototype.replace` substitution patterns
		return full.replace(inner, () => rewritten);
	});

/**
 * Rewrites the relative `.ts` import specifiers of one file of `dist` to `.js`. A `.js`
 * or `.svelte` file changes only at its import requests, a `.d.ts` file by pattern.
 *
 * @param path - the file's path, whose extension picks the rewrite and which names it in the warning
 * @param content - the file's text
 * @param log - warned of a `.js` or `.svelte` file that doesn't parse and is rewritten by pattern
 * @returns the rewritten text
 */
export const rewrite_dist_file_imports = (path: string, content: string, log?: Logger): string => {
	// TODO: if tsv reports type-position `import()` spans, `.d.ts` can use them too, and `.svelte`
	// scripts can rewrite theirs
	if (path.endsWith('.d.ts')) return rewrite_relative_ts_imports(content);

	const is_svelte = path.endsWith('.svelte');
	const parsed = is_svelte ? parse_imports_svelte(content) : parse_imports_typescript(content);
	if (!parsed.ok) {
		log?.warn(
			`rewriting .ts imports in ${path} by pattern, which also reaches its strings and comments, because it doesn't parse: ${parsed.error.message}`
		);
		return is_svelte ? rewrite_svelte_ts_imports(content) : rewrite_relative_ts_imports(content);
	}

	// tsv's Svelte offsets are after its documented removal of a leading BOM, which `read_text` keeps
	const offset = is_svelte && content.charCodeAt(0) === 0xfeff ? 1 : 0;
	let next = '';
	let cursor = 0;
	// the records are in source order
	for (const { specifier } of parsed.imports) {
		if (!specifier) continue;
		const start = specifier.start + offset;
		const end = specifier.end + offset;
		if (!RELATIVE_TS_SPECIFIER_MATCHER.test(content.slice(start, end))) continue;
		next += content.slice(cursor, end - 2) + 'js';
		cursor = end;
	}
	return next + content.slice(cursor);
};

export interface RewriteDistImportsResult {
	/** Number of files scanned (`.js`, `.d.ts`, and `.svelte`). */
	scanned: number;
	/** Number of scanned files whose contents changed. */
	rewritten: number;
}

/** Bounds open file descriptors while rewriting the dist tree. */
const DIST_REWRITE_CONCURRENCY = 16;

/**
 * Walks `dist_dir` and rewrites relative `.ts` import specifiers to `.js` in every
 * `.js` runtime file, `.d.ts` declaration file, and `.svelte` file, as
 * `rewrite_dist_file_imports` does. Source maps (`*.map`) and symlinks are skipped.
 *
 * @param fs_paths - opens `dist_dir`, its symlinks followed, beneath which the files are rewritten
 * @param dist_dir - the packaged output directory, absolute or relative to the `fs_paths` project;
 * defaults to `dist`
 * @param log - logs the counts, and warns of a file that doesn't parse
 * @returns counts of files scanned and rewritten
 */
export const rewrite_dist_imports = async (
	fs_paths: FsPaths,
	dist_dir: string = SVELTEKIT_DIST_DIRNAME,
	log?: Logger
): Promise<RewriteDistImportsResult> => {
	// opened once, the found files read and written beneath it
	const dist = await fs_paths.open_dir_path(dist_dir);
	if (dist === null) return { scanned: 0, rewritten: 0 };
	// regular files only, a symlink never followed into what it points at
	const found = (
		await fs_search(dist.dir, {
			path: dist.path,
			id_dir: resolve(fs_paths.project.path, dist_dir),
			file_filter: (id) => id.endsWith('.js') || id.endsWith('.d.ts') || id.endsWith('.svelte')
		})
	).filter((f) => f.kind === 'file');

	const changed = await map_concurrent(found, DIST_REWRITE_CONCURRENCY, async ({ path }) => {
		const file_path = dist.path ? dist.path + '/' + path : path;
		const content = await dist.dir.read_text(file_path);
		const next = rewrite_dist_file_imports(file_path, content, log);
		if (next === content) return false;
		await dist.dir.write(file_path, next);
		// log?.debug(`rewrote relative .ts import specifiers to .js in ${path}`);
		return true;
	});
	const rewritten = changed.filter(Boolean).length;

	if (found.length) {
		log?.debug(
			`rewrote relative .ts→.js import specifiers in ${rewritten}/${found.length} dist files`
		);
	}

	return { scanned: found.length, rewritten };
};
