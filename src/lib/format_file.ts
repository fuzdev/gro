import {format_css, format_svelte, format_typescript} from '@fuzdev/tsv_wasm';
import {extname} from 'node:path';

/**
 * The source languages Gro can format in-process, backed by
 * `@fuzdev/tsv_wasm` (`typescript`/`svelte`/`css`) plus a builtin `json`
 * formatter. Anything else passes through unchanged.
 */
export type FormatLang = 'typescript' | 'svelte' | 'css' | 'json';

export interface FormatFileOptions {
	/** The file path, used to infer the language from its extension. */
	filepath?: string;
	/** The language to format as, overriding any inference from `filepath`. */
	lang?: FormatLang;
}

/**
 * Formats a string of source code in-process.
 * Passes the input through unchanged when the language is unsupported (an
 * expected no-op), but throws when the formatter rejects the source — e.g. a
 * syntax error — so callers can decide whether to log, skip, or fail rather
 * than silently treating broken input as already-formatted.
 * @param content - the source to format
 * @param options - a `filepath` to infer the language, or an explicit `lang`
 */
export const format_file = (content: string, options: FormatFileOptions = {}): string => {
	const lang = options.lang ?? (options.filepath ? infer_lang(options.filepath) : null);
	switch (lang) {
		case 'typescript':
			return format_typescript(content);
		case 'svelte':
			return format_svelte(content);
		case 'css':
			return format_css(content);
		case 'json':
			return format_json(content);
		default:
			return content;
	}
};

/**
 * Formats JSON with tabs.
 * Passes non-strict JSON (e.g. JSONC like `tsconfig.json` with comments)
 * through unchanged rather than throwing — unparseable JSON is usually
 * intentional, not a formatting error.
 * Forward-compatible with a future `tsv` JSON formatter.
 */
const format_json = (content: string): string => {
	let parsed;
	try {
		parsed = JSON.parse(content);
	} catch (_err) {
		return content;
	}
	return JSON.stringify(parsed, null, '\t') + '\n';
};

/**
 * Infers the format language from a file path's extension.
 * Returns `null` for extensions Gro doesn't format.
 */
const infer_lang = (path: string): FormatLang | null => {
	switch (extname(path).substring(1)) {
		case 'ts':
		case 'mts':
		case 'cts':
		case 'js':
		case 'mjs':
		case 'cjs': {
			return 'typescript';
		}
		case 'svelte': {
			return 'svelte';
		}
		case 'css': {
			return 'css';
		}
		case 'json': {
			return 'json';
		}
		default: {
			return null;
		}
	}
};
