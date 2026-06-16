// Followup sweep helper for the `.ts` import-extensions quest (NOT shipped — uncommitted).
//
// Inverts the forward codemod's category-3 relativization: restores SvelteKit
// `$lib` / `$routes` aliases in APP code (`src/routes`, `src/test`) that the forward
// pass dissolved into relative paths. Library code (`src/lib`) stays relative — the
// convention is that aliases aren't used there.
//
// This is the companion to dropping `rewriteRelativeImportExtensions` (back to
// `false`): with the flag off, a non-relative `$lib/foo.ts` is no longer a hard
// error, so the aliases can come back with their real `.ts` source extension. The
// gro dist-rewrite pass (now over `.js` too) owns the `.ts`→`.js` rewrite for
// shipped `dist`; `src/routes` / `src/test` aren't published, so their `.ts`
// specifiers only need to resolve in dev/build (Vite resolves `$lib`/`$routes` + `.ts`).
//
// Rewrite rule (per real Import/Export/ImportExpression specifier, relative only):
//   - resolves into `<repo>/src/lib`            -> `$lib/<rest>`     (always; lib is a
//                                                   different tree, app code aliases it)
//   - resolves into `<repo>/src/routes` via `../` (escapes the file's own dir)
//                                                -> `$routes/<rest>` (was an alias)
//   - same-dir `./x` into `src/routes`           -> left relative    (co-located idiom)
//   - anything else (into `src/test`, sibling)   -> left relative
// The real source extension (`.ts` / `.svelte.ts` / `.svelte`) is preserved.
//
// Parses with tsv (Acorn/ESTree) + zimmerframe, edits by byte-range splice of the
// specifier literal — so only real module specifiers change, never comment/string text.
//
// Usage (dry-run preview by default; pass --write to apply):
//   node codemod_restore_aliases.mjs <repo_root> [--dir src/routes] [--dir src/test] [--write]
// Defaults to `--dir src/routes --dir src/test` (never `src/lib`).

import {readFileSync, writeFileSync, readdirSync, statSync, existsSync} from 'node:fs';
import {join, relative, dirname, resolve, sep} from 'node:path';

import {parse_typescript, parse_svelte} from '../tsv/crates/tsv_wasm/pkg/parse/npm/index.js';
import {walk} from 'zimmerframe';

// ---- args ----------------------------------------------------------------
const argv = process.argv.slice(2);
const positionals = [];
const dirs = [];
let write = false;
for (let i = 0; i < argv.length; i++) {
	const a = argv[i];
	if (a === '--write') write = true;
	else if (a === '--dir') dirs.push(argv[++i]);
	else positionals.push(a);
}
const repo_root = resolve(positionals[0] ?? '.');
if (!dirs.length) dirs.push('src/routes', 'src/test');

const LIB_DIR = join(repo_root, 'src/lib');
const ROUTES_DIR = join(repo_root, 'src/routes');

// ---- specifier rewrite ---------------------------------------------------
/** @returns the re-aliased specifier, or null to leave unchanged */
const rewrite_specifier = (spec, file_dir) => {
	if (!/^\.\.?\//.test(spec)) return null; // only relativized specifiers
	const abs = resolve(file_dir, spec);
	// into src/lib -> `$lib/...` (always — app code aliases the library tree)
	if (abs === LIB_DIR || abs.startsWith(LIB_DIR + sep)) {
		const rest = relative(LIB_DIR, abs).split(sep).join('/');
		return '$lib/' + rest;
	}
	// into src/routes via `../` (escapes the file's own dir) -> `$routes/...`;
	// a same-dir `./x` is left as a co-located relative import (idiomatic)
	if (spec.startsWith('../') && (abs === ROUTES_DIR || abs.startsWith(ROUTES_DIR + sep))) {
		const rest = relative(ROUTES_DIR, abs).split(sep).join('/');
		return '$routes/' + rest;
	}
	return null;
};

// ---- AST walk: collect specifier-literal edits ---------------------------
const collect_edits = (ast, file_dir) => {
	const edits = [];
	const consider = (lit) => {
		if (!lit || lit.type !== 'Literal' || typeof lit.value !== 'string') return;
		const next = rewrite_specifier(lit.value, file_dir);
		if (next !== null && next !== lit.value) {
			const quote = lit.raw[0];
			edits.push({start: lit.start, end: lit.end, text: quote + next + quote});
		}
	};
	walk(ast, null, {
		ImportDeclaration(node, ctx) {
			consider(node.source);
			ctx.next();
		},
		ExportNamedDeclaration(node, ctx) {
			consider(node.source);
			ctx.next();
		},
		ExportAllDeclaration(node, ctx) {
			consider(node.source);
			ctx.next();
		},
		ImportExpression(node, ctx) {
			consider(node.source);
			ctx.next();
		},
	});
	return edits;
};

// ---- driver --------------------------------------------------------------
const walk_files = (dir) => {
	const out = [];
	for (const name of readdirSync(dir)) {
		const id = join(dir, name);
		if (statSync(id).isDirectory()) out.push(...walk_files(id));
		else if (/\.(ts|svelte)$/.test(name)) out.push(id);
	}
	return out;
};

let files_changed = 0;
let edits_total = 0;
const failures = [];
for (const d of dirs) {
	const abs_dir = resolve(repo_root, d);
	if (!existsSync(abs_dir)) continue;
	for (const file of walk_files(abs_dir)) {
		const source = readFileSync(file, 'utf8');
		let ast;
		try {
			ast = file.endsWith('.svelte') ? parse_svelte(source) : parse_typescript(source);
		} catch (err) {
			failures.push(`${relative(repo_root, file)}: ${err.message ?? err}`);
			continue;
		}
		const edits = collect_edits(ast, dirname(file));
		if (!edits.length) continue;
		edits_total += edits.length;
		files_changed++;
		let next = source;
		for (const e of edits.sort((a, b) => b.start - a.start)) {
			next = next.slice(0, e.start) + e.text + next.slice(e.end);
		}
		if (write) writeFileSync(file, next);
		else console.log(`  ${relative(repo_root, file)}: ${edits.length} specifier(s)`);
	}
}

console.log(
	`${write ? 'rewrote' : '[dry-run] would rewrite'} ${edits_total} specifiers across ${files_changed} files` +
		(write ? '' : ' (pass --write to apply)'),
);
if (failures.length) {
	console.warn(`\n${failures.length} parse failure(s):`);
	for (const f of failures) console.warn(`  ${f}`);
}
