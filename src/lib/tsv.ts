/**
 * The tsv host module: tsv's formatters for TypeScript, Svelte, and CSS, its import-records
 * parse, and its gitignore-style matcher, reaching nothing.
 *
 * A formatter takes the source and returns `{ok: true, code}`, or `{ok: false, error}` for a
 * source it can't format: one that doesn't parse (`kind: 'parse'`), or, from `format_svelte`
 * alone, one that parses but can't be printed without changing its meaning (`kind:
 * 'refused'`). An import parse returns `{ok: true, hasModuleSyntax, imports}`, every request in
 * source order, or the same `{ok: false, error}` for a source that doesn't parse. An error is a
 * plain object: its one-line `message`, the `frame` under it (`${message}\n${frame}` is what the
 * `tsv` CLI prints), and its span as UTF-16 offsets, `start` and `end`, and their `loc`, a
 * 1-based line and a 0-based column. Every offset indexes the caller's source, but a Svelte
 * document's, which index it with a leading byte-order mark removed.
 *
 * What a function refuses it throws as a `TypeError`, its text tsv's own: a source that isn't a
 * string or isn't well-formed UTF-16, and options that aren't an object, name a key the
 * function doesn't take, or hold a `sourceType` that isn't `'script'` or `'module'`, a key
 * TypeScript's functions alone take. Options are read as data, so no code of a caller's runs: a
 * proxy is refused as options that aren't an object, and a `sourceType` that is a getter by a
 * text of Gro's own. With no `sourceType`, TypeScript is
 * read as a module, retried as a script when that fails to parse; a set one is exact.
 *
 * An ignore stack matches paths, relative to a root and `/`-separated, against gitignore-style
 * layers, each anchored at a directory relative to the same root (`''` the root itself) and
 * pushed shallowest-first: `.gitignore` layers, then override layers (`.formatignore`, or a
 * `.prettierignore` read in its place) applied after every `.gitignore` one. Precedence is push
 * order, so a deeper layer pushed before a shallower one loses to it, where git would have the
 * deeper one win. A stack is frozen,
 * null-prototype, and recognized by its methods, which throw a `TypeError` called on any other
 * object, and lives as long as it can be reached, with no call to end it.
 *
 * This is the Node implementation, which runs on the Node Gro, over tsv's N-API binding,
 * `@fuzdev/tsv`. On `fgro` the module is the runtime's own, over tsv's crates, and grant-free, so
 * the project and every package may import it.
 *
 * @module
 */

import {
	format_css as tsv_format_css,
	format_svelte as tsv_format_svelte,
	format_typescript as tsv_format_typescript,
	IgnoreStack,
	parse_imports_svelte as tsv_parse_imports_svelte,
	parse_imports_typescript as tsv_parse_imports_typescript
} from '@fuzdev/tsv';
import { isProxy } from 'node:util/types';

import { freeze_methods } from './internal/records.ts';

const { freeze, getOwnPropertyDescriptor, keys, setPrototypeOf } = Object;
const { isArray } = Array;
const { stringify } = JSON;

/**
 * The parse goal of TypeScript's functions: at `'script'`, `await` is an identifier and
 * `import`, `export`, and `import.meta` are syntax errors, and the source is sloppy unless its
 * own `"use strict"` makes it strict.
 */
export type TsvSourceType = 'script' | 'module';

/**
 * The options of TypeScript's functions.
 */
export interface TsvTypeScriptOptions {
	/** The parse goal: exact when set, else a module retried as a script. */
	sourceType?: TsvSourceType | undefined;
}

/**
 * The options of the Svelte and CSS functions, which take no settable key: `sourceType` is
 * refused unless it's `undefined`, so one bag can be passed to any function.
 */
export interface TsvOptions {
	sourceType?: undefined;
}

/**
 * A point of a source: a 1-based line and a 0-based column in UTF-16 code units.
 */
export interface TsvPosition {
	line: number;
	column: number;
}

/**
 * Why a source didn't format or parse: `'parse'`, a syntax error, whose `end` is its `start`, or
 * `'refused'`, `format_svelte`'s refusal of a lone CR inside an in-tag `//` comment, which
 * reformatting would turn into a line break that ends the comment early, its span the CR.
 */
export interface TsvError {
	kind: 'parse' | 'refused';
	/** What went wrong, on one line. */
	message: string;
	/** The `line:column` header and the source line around the error, then a caret under it. */
	frame: string;
	/** The UTF-16 offset the error starts at. */
	start: number;
	/** The UTF-16 offset its span ends at. */
	end: number;
	/** `start` and `end` as points. */
	loc: { start: TsvPosition; end: TsvPosition };
}

/**
 * A source that didn't format or parse.
 */
export interface TsvFailure {
	ok: false;
	error: TsvError;
}

/**
 * What a formatter returns: the formatted source, the same string when it was formatted already,
 * or why it didn't format.
 */
export type TsvFormatResult = { ok: true; code: string } | TsvFailure;

/**
 * A span of a source, as UTF-16 offsets.
 */
export interface TsvSpan {
	start: number;
	end: number;
}

/**
 * One module request.
 */
export interface TsvImportRecord extends TsvSpan {
	/**
	 * An `import` declaration, an `export ... from`, an `import()`, or an `import x =
	 * require('y')`.
	 */
	kind: 'static' | 're_export' | 'dynamic' | 'import_equals';
	/** The specifier's decoded value and span, `null` for a computed `import()`. */
	specifier: (TsvSpan & { value: string }) | null;
	/** `'statement'` for `import type`, `'specifiers'` when every specifier is `type`. */
	typeOnly: null | 'statement' | 'specifiers';
	/** The import's phase, of the source-phase and deferred-import proposals. */
	phase: null | 'source' | 'defer';
	/** The span of its `with { ... }` attributes. */
	attributes: TsvSpan | null;
}

/**
 * What an import parse returns: the requests and whether the source uses syntax only a module
 * parses, or why it didn't parse.
 */
export type TsvImportsResult =
	{ ok: true; hasModuleSyntax: boolean; imports: Array<TsvImportRecord> } | TsvFailure;

/**
 * A stack of gitignore-style layers matching paths relative to its root.
 */
export interface TsvIgnoreStack {
	/**
	 * Pushes a `.gitignore`'s `content`, anchored at the directory `anchor`.
	 * @throws `TypeError` - for an `anchor` or `content` that isn't a string
	 */
	push_gitignore(anchor: string, content: string): void;
	/** Pops the last `.gitignore` layer pushed, if any. */
	pop_gitignore(): void;
	/**
	 * Pushes a `.formatignore`'s `content` as an override layer, anchored at `anchor`.
	 * @throws `TypeError` - for an `anchor` or `content` that isn't a string
	 */
	push_formatignore(anchor: string, content: string): void;
	/**
	 * Pushes a `.prettierignore`'s `content` as an override layer, anchored at `anchor`, matched
	 * as a `.formatignore`'s.
	 * @throws `TypeError` - for an `anchor` or `content` that isn't a string
	 */
	push_prettierignore(anchor: string, content: string): void;
	/** Pops the last override layer pushed, if any. */
	pop_override(): void;
	/**
	 * Whether `path` is ignored, `is_dir` marking a directory, so a pattern ending in `/` applies.
	 * @throws `TypeError` - for a `path` that isn't a string or an `is_dir` that isn't a boolean
	 */
	is_ignored(path: string, is_dir: boolean): boolean;
}

/** The family a function's refusals name. */
type Noun = 'format' | 'imports';

/**
 * `source`, or a `TypeError` when it isn't a string or holds a lone surrogate, which the
 * binding would read as U+FFFD.
 */
const read_source = (source: unknown, noun: Noun): string => {
	if (typeof source !== 'string') {
		throw new TypeError(
			`${noun} source must be a string (got ${source === null ? 'null' : typeof source})`
		);
	}
	if (!source.isWellFormed()) {
		throw new TypeError(
			`${noun} source must be well-formed UTF-16 (a lone surrogate at offset ${lone_surrogate_offset(source)})`
		);
	}
	return source;
};

/** The offset of the first lone surrogate in `source`, which holds one. */
const lone_surrogate_offset = (source: string): number => {
	for (let i = 0; i < source.length; i++) {
		const unit = source.charCodeAt(i);
		if (unit < 0xd800 || unit > 0xdfff) continue;
		if (unit <= 0xdbff) {
			const next = source.charCodeAt(i + 1);
			if (next >= 0xdc00 && next <= 0xdfff) {
				i++;
				continue;
			}
		}
		return i;
	}
	return -1;
};

/**
 * The source type `options` names, read as data, or a `TypeError` for options that aren't an
 * object, are a proxy, or hold a key the function doesn't take or one that's a getter.
 * @param typescript - whether the function is TypeScript's, the one that takes `sourceType`
 */
const read_options = (
	options: unknown,
	noun: Noun,
	typescript: boolean
): TsvSourceType | undefined => {
	if (options === undefined || options === null) return undefined;
	if (typeof options !== 'object' || isArray(options) || isProxy(options)) {
		throw new TypeError(`${noun} options must be an object`);
	}
	let source_type: TsvSourceType | undefined;
	for (const name of keys(options)) {
		if (name !== 'sourceType') {
			const expected = typescript ? "expected 'sourceType'" : 'this export takes no options';
			throw new TypeError(`unknown ${noun} option ${quote_string(name)} (${expected})`);
		}
		const descriptor = getOwnPropertyDescriptor(options, name)!;
		if (!('value' in descriptor)) {
			throw new TypeError(`${noun} option 'sourceType' must be data, not a getter`);
		}
		const { value } = descriptor;
		if (value === undefined) continue;
		if (!typescript) {
			throw new TypeError(`${noun} option 'sourceType' is only supported for TypeScript`);
		}
		if (value !== 'script' && value !== 'module') {
			throw new TypeError(
				`${noun} option 'sourceType' must be 'script' or 'module' (got ${describe_value(value)})`
			);
		}
		source_type = value;
	}
	return source_type;
};

/** A refused value as a message names it, as tsv's options reader does. */
const describe_value = (value: unknown): string => {
	if (typeof value === 'string') return quote_string(value);
	if (typeof value === 'number' || typeof value === 'boolean') return String(value);
	return value === null ? 'null' : typeof value;
};

/** The longest string `quote_string` echoes whole, in UTF-16 units. */
const QUOTED_MAX = 40;

/** What JSON leaves raw that a terminal reads as a line break, a control, or a reordering. */
const TERMINAL_UNSAFE = /[\u007f-\u009f\u061c\u2028\u2029\u200e\u200f\u202a-\u202e\u2066-\u2069]/g;

/**
 * `value` single-quoted, escaped as `JSON.stringify` escapes it, `'` escaped and `"` not, with
 * `\uXXXX` for `TERMINAL_UNSAFE`, and cut to `QUOTED_MAX` units, one sooner where the cut would
 * split a surrogate pair, with a `…`, as tsv's options reader quotes it.
 */
const quote_string = (value: string): string => {
	let cut = value.length;
	if (cut > QUOTED_MAX) {
		cut = QUOTED_MAX;
		const last = value.charCodeAt(cut - 1);
		if (last >= 0xd800 && last <= 0xdbff) cut--;
	}
	const body = stringify(value.slice(0, cut))
		.slice(1, -1)
		.replace(/\\"/g, '"')
		.replace(/'/g, "\\'")
		.replace(TERMINAL_UNSAFE, (c) => `\\u${c.charCodeAt(0).toString(16).padStart(4, '0')}`);
	return `'${body}${cut < value.length ? '…' : ''}'`;
};

/** The bag the binding is handed: the checked source type, or none. */
const to_bag = (source_type: TsvSourceType | undefined) =>
	source_type === undefined ? undefined : { sourceType: source_type };

/**
 * Formats TypeScript or JavaScript.
 * @throws `TypeError` - for a source that isn't a well-formed string, or options it refuses
 */
export const format_typescript = (
	source: string,
	options?: TsvTypeScriptOptions
): TsvFormatResult => {
	const text = read_source(source, 'format');
	return tsv_format_typescript(text, to_bag(read_options(options, 'format', true)));
};

/**
 * Formats a Svelte component.
 * @throws `TypeError` - for a source that isn't a well-formed string, or options it refuses
 */
export const format_svelte = (source: string, options?: TsvOptions): TsvFormatResult => {
	const text = read_source(source, 'format');
	read_options(options, 'format', false);
	return tsv_format_svelte(text);
};

/**
 * Formats CSS.
 * @throws `TypeError` - for a source that isn't a well-formed string, or options it refuses
 */
export const format_css = (source: string, options?: TsvOptions): TsvFormatResult => {
	const text = read_source(source, 'format');
	read_options(options, 'format', false);
	return tsv_format_css(text);
};

/**
 * The import requests of TypeScript or JavaScript.
 * @throws `TypeError` - for a source that isn't a well-formed string, or options it refuses
 */
export const parse_imports_typescript = (
	source: string,
	options?: TsvTypeScriptOptions
): TsvImportsResult => {
	const text = read_source(source, 'imports');
	return tsv_parse_imports_typescript(text, to_bag(read_options(options, 'imports', true)));
};

/**
 * The import requests of a Svelte component's `<script>` blocks, in source order.
 * @throws `TypeError` - for a source that isn't a well-formed string, or options it refuses
 */
export const parse_imports_svelte = (source: string, options?: TsvOptions): TsvImportsResult => {
	const text = read_source(source, 'imports');
	read_options(options, 'imports', false);
	return tsv_parse_imports_svelte(text);
};

const NOT_A_STACK = "an ignore stack's methods are called on an ignore stack, not another object";
const NOT_A_LAYER = "an ignore layer's anchor and content are strings";
const NOT_A_PATH = 'is_ignored takes a path string and an is_dir boolean';

// the key only this module holds, so a stack's `constructor` can't make another stack
const STACK_KEY = Symbol('tsv ignore stack');

class NodeIgnoreStack implements TsvIgnoreStack {
	readonly #inner: IgnoreStack;

	constructor(key: symbol) {
		if (key !== STACK_KEY) {
			throw new TypeError('ignore stacks come from create_ignore_stack, not a constructor');
		}
		this.#inner = new IgnoreStack();
	}

	/**
	 * The matcher of `stack`, or a `TypeError` when it's no stack: a proxy of one is none.
	 */
	static #inner_of(stack: unknown): IgnoreStack {
		if (typeof stack !== 'object' || stack === null || !(#inner in stack)) {
			throw new TypeError(NOT_A_STACK);
		}
		return stack.#inner;
	}

	/** The matcher of `stack`, with `anchor` and `content` checked. */
	static #layer_of(stack: unknown, anchor: unknown, content: unknown): IgnoreStack {
		const inner = NodeIgnoreStack.#inner_of(stack);
		if (typeof anchor !== 'string' || typeof content !== 'string') {
			throw new TypeError(NOT_A_LAYER);
		}
		return inner;
	}

	push_gitignore(anchor: string, content: string): void {
		NodeIgnoreStack.#layer_of(this, anchor, content).push_gitignore(anchor, content);
	}

	pop_gitignore(): void {
		NodeIgnoreStack.#inner_of(this).pop_gitignore();
	}

	push_formatignore(anchor: string, content: string): void {
		NodeIgnoreStack.#layer_of(this, anchor, content).push_formatignore(anchor, content);
	}

	push_prettierignore(anchor: string, content: string): void {
		NodeIgnoreStack.#layer_of(this, anchor, content).push_prettierignore(anchor, content);
	}

	pop_override(): void {
		NodeIgnoreStack.#inner_of(this).pop_override();
	}

	is_ignored(path: string, is_dir: boolean): boolean {
		const inner = NodeIgnoreStack.#inner_of(this);
		if (typeof path !== 'string' || typeof is_dir !== 'boolean') {
			throw new TypeError(NOT_A_PATH);
		}
		return inner.is_ignored(path, is_dir);
	}
}

// frozen, null-prototype, so a polluted `Object.prototype.then` can't make `await` adopt a forged value
setPrototypeOf(NodeIgnoreStack.prototype, null);
freeze_methods(NodeIgnoreStack.prototype);
freeze(NodeIgnoreStack);

/**
 * A new ignore stack, which ignores nothing until a layer is pushed.
 */
export const create_ignore_stack = (): TsvIgnoreStack => freeze(new NodeIgnoreStack(STACK_KEY));

freeze(format_typescript);
freeze(format_svelte);
freeze(format_css);
freeze(parse_imports_typescript);
freeze(parse_imports_svelte);
freeze(create_ignore_stack);
