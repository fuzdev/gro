import { parseSync, Visitor, type Expression, type VisitorObject } from 'oxc-parser';
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

/**
 * Collects the import specifiers of a JS, TS, or Svelte module: static imports, dynamic and
 * (when types are kept) type-position imports with a single-string source, and re-exports, each
 * group in source order.
 *
 * @param id - the module's path, which picks the parser
 * @param contents - the module's source
 * @param ignore_types - skip type-only imports and re-exports; when false, type-position
 *   imports (`typeof import('./x')`, `import('./t').X`) are collected too
 * @returns the specifiers, decoded, duplicates kept
 */
export const parse_imports = (
	id: PathId,
	contents: string,
	ignore_types = true
): Array<ImportSpecifier> => {
	const specifiers: Array<string> = [];
	const is_svelte = SVELTE_MATCHER.test(id);

	const parse_from = (s: string): void => {
		const parsed = parseSync(is_svelte ? id + '.ts' : id, s, {});

		// Process static imports, one pass over the top-level statements
		for (const node of parsed.program.body) {
			if (node.type !== 'ImportDeclaration') continue;

			// the parser's decoded value, exact for every escape
			const value = node.source.value;

			if (!value) continue;

			// Skip type-only imports if ignore_types is true
			if (ignore_types) {
				// Handle import type {...} (type-only imports)
				if (node.importKind === 'type') {
					continue;
				}

				// Handle inline type imports ({type foo}), skipped when every specifier is one
				if (
					node.specifiers.length > 0 &&
					node.specifiers.every((s) => s.type === 'ImportSpecifier' && s.importKind === 'type')
				) {
					continue;
				}
			}

			specifiers.push(value);
		}

		// Process dynamic imports anywhere in the module, from the parser's decoded values,
		// and when types are kept, type-position imports (`typeof import('./x')`, `import('./t').X`)
		const visitor: VisitorObject = {
			ImportExpression: (node) => {
				const value = to_import_expression_value(node.source);
				if (value) specifiers.push(value);
			}
		};
		if (!ignore_types) {
			visitor.TSImportType = (node) => {
				const { argument } = node;
				if (argument.type === 'TSLiteralType' && argument.literal.type === 'Literal') {
					const { value } = argument.literal;
					if (typeof value === 'string' && value) specifiers.push(value);
				}
			};
		}
		new Visitor(visitor).visit(parsed.program);

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
