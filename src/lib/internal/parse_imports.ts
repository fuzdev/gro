import { parseSync, Visitor, type Expression, type ImportDeclaration } from 'oxc-parser';
import type { Flavored } from '@fuzdev/fuz_schema/schema.ts';
import type { PathId } from '@fuzdev/fuz_util/path.ts';

import { JS_MATCHER, TS_MATCHER, SVELTE_MATCHER, SVELTE_SCRIPT_MATCHER } from '../constants.ts';

export type ImportSpecifier = Flavored<string, 'ImportSpecifier'>;

/**
 * The decoded value of a dynamic import's source when it's a single string:
 * a string literal or a template literal without substitutions.
 */
const to_import_expression_value = (source: Expression): string | null => {
	if (source.type === 'Literal') return typeof source.value === 'string' ? source.value : null;
	if (source.type === 'TemplateLiteral' && source.expressions.length === 0) {
		return source.quasis[0]?.value.cooked ?? null;
	}
	return null;
};

export const parse_imports = (
	id: PathId,
	contents: string,
	ignore_types = true
): Array<ImportSpecifier> => {
	const specifiers: Array<string> = [];
	const is_svelte = SVELTE_MATCHER.test(id);

	const parse_from = (s: string): void => {
		const parsed = parseSync(is_svelte ? id + '.ts' : id, s, {});

		// Process static imports
		for (const static_import of parsed.module.staticImports) {
			// Get the module source node
			const import_decl = parsed.program.body.find(
				(node) => node.type === 'ImportDeclaration' && node.start === static_import.start
			) as ImportDeclaration | undefined;

			if (!import_decl?.source) continue;

			// the parser's decoded value, exact for every escape
			const value = import_decl.source.value;

			if (!value) continue;

			// Skip type-only imports if ignore_types is true
			if (ignore_types) {
				// Handle import type {...} (type-only imports)
				if (import_decl.importKind === 'type') {
					continue;
				}

				// Handle inline type imports ({type foo})
				if (static_import.entries.length > 0) {
					// If all imports are type imports, skip this import
					const has_non_type_specifier = static_import.entries.some((entry) => !entry.isType);
					if (!has_non_type_specifier) {
						continue;
					}
				}
			}

			specifiers.push(value);
		}

		// Process dynamic imports anywhere in the module, from the parser's decoded values
		new Visitor({
			ImportExpression: (node) => {
				const value = to_import_expression_value(node.source);
				if (value) specifiers.push(value);
			}
		}).visit(parsed.program);

		// Process re-exports, `export {x} from` and `export * from`
		for (const node of parsed.program.body) {
			if (
				(node.type === 'ExportNamedDeclaration' || node.type === 'ExportAllDeclaration') &&
				node.source
			) {
				// Skip type-only exports if ignore_types is true
				if (ignore_types && node.exportKind === 'type') {
					continue;
				}
				const value = node.source.value;
				if (value) {
					specifiers.push(value);
				}
			}
		}
	};

	if (is_svelte) {
		// Reset the regexp state between calls
		SVELTE_SCRIPT_MATCHER.lastIndex = 0;

		// Capture script tags at the top level (not nested in HTML)
		let last_index = 0;
		const script_blocks: Array<{ content: string; start: number; end: number }> = [];

		// First collect all script blocks
		let match;
		while ((match = SVELTE_SCRIPT_MATCHER.exec(contents)) !== null) {
			// Save position of the script tag
			const start = match.index;
			const end = SVELTE_SCRIPT_MATCHER.lastIndex;

			// Only process top-level script tags (skip nested ones)
			// A nested script would be inside another HTML tag between lastIndex and start
			const text_between = contents.substring(last_index, start);
			const contains_opening_tag = /<[a-z][^>]*>/i.test(text_between);
			const contains_closing_tag = /<\/[a-z][^>]*>/i.test(text_between);

			// If we're not nested (no HTML tag nesting), process this script
			if (!(contains_opening_tag && !contains_closing_tag)) {
				script_blocks.push({
					content: match[1]!,
					start,
					end
				});
			}

			last_index = end;
		}

		// Process all the collected script blocks
		for (const script_block of script_blocks) {
			parse_from(script_block.content);
		}
	} else if (TS_MATCHER.test(id) || JS_MATCHER.test(id)) {
		parse_from(contents);
	}

	return specifiers;
};
