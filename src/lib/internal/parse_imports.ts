import {
	parse_imports_svelte,
	parse_imports_typescript,
	type ImportsResult,
	type TsvParseError
} from '@fuzdev/tsv';
import type { Flavored } from '@fuzdev/fuz_schema/schema.ts';
import type { Result } from '@fuzdev/fuz_util/result.ts';
import type { PathId } from '@fuzdev/fuz_util/path.ts';

import { JS_MATCHER, TS_MATCHER, SVELTE_MATCHER } from '../constants.ts';

export type ImportSpecifier = Flavored<string, 'ImportSpecifier'>;

/**
 * Collects the value import specifiers of a JS, TS, or Svelte module in source order: static
 * imports, re-exports, `import x = require()`, and dynamic imports whose source is a single
 * string (a string literal or a template literal without substitutions). A Svelte
 * component's are those of its `<script>` blocks, not its markup's expressions. Type-only
 * requests are skipped, both `import type` and those whose every specifier is an inline `type`
 * one, and type positions (`typeof import('./x')`) hold none. Any other file has none.
 *
 * @param id - the module's path, which picks the parser
 * @param contents - the module's source
 * @returns the specifiers, decoded, empty ones skipped, duplicates kept, or tsv's parse error
 *   when the source doesn't parse
 */
export const parse_imports = (
	id: PathId,
	contents: string
): Result<{ specifiers: Array<ImportSpecifier> }, { error: TsvParseError }> => {
	let result: ImportsResult;
	if (SVELTE_MATCHER.test(id)) {
		// TODO a dynamic import in the markup (`{#await import('./x.ts')}`) is a dependency too,
		// but `parse_imports_svelte` reads only the `<script>` blocks
		result = parse_imports_svelte(contents);
	} else if (TS_MATCHER.test(id) || JS_MATCHER.test(id)) {
		result = parse_imports_typescript(contents);
	} else {
		return { ok: true, specifiers: [] };
	}
	if (!result.ok) return { ok: false, error: result.error };
	const specifiers: Array<ImportSpecifier> = [];
	for (const record of result.imports) {
		if (record.typeOnly !== null) continue;
		const value = record.specifier?.value;
		if (value) specifiers.push(value);
	}
	return { ok: true, specifiers };
};
