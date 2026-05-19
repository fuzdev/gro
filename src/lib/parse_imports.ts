import {parseSync, type ImportDeclaration} from 'oxc-parser';
import type {Flavored} from '@fuzdev/fuz_util/types.js';
import {UnreachableError} from '@fuzdev/fuz_util/error.js';
import type {PathId} from '@fuzdev/fuz_util/path.js';

import {JS_MATCHER, TS_MATCHER, SVELTE_MATCHER, SVELTE_SCRIPT_MATCHER} from './constants.ts';

export type ImportSpecifier = Flavored<string, 'ImportSpecifier'>;

// TODO this is probably way more complicated that it should be, maybe report the issues upstream unless I made a mistake here

/**
 * Extracts the string value from a module request, handling different quote styles.
 * Returns null if the value is not a valid string literal.
 */
const extract_string_literal = (content: string, start: number, end: number): string | null => {
	const value = content.substring(start, end);

	// Check if it's a string literal (starts and ends with quotes)
	if (
		(value.startsWith("'") && value.endsWith("'")) ||
		(value.startsWith('"') && value.endsWith('"')) ||
		(value.startsWith('`') && value.endsWith('`'))
	) {
		// Remove the quotes
		return value.slice(1, -1);
	}

	// Not a valid string literal
	return null;
};

export const parse_imports = (
	id: PathId,
	contents: string,
	ignore_types = true,
): Array<ImportSpecifier> => {
	const specifiers: Array<string> = [];
	const is_svelte = SVELTE_MATCHER.test(id);

	const parse_from = (s: string): void => {
		const parsed = parseSync(is_svelte ? id + '.ts' : id, s, {});

		// Process static imports
		for (const static_import of parsed.module.staticImports) {
			// Get the module source node
			const import_decl = parsed.program.body.find(
				(node) => node.type === 'ImportDeclaration' && node.start === static_import.start,
			) as ImportDeclaration | undefined;

			if (!import_decl?.source) continue;

			// Extract the module request string value
			const value = extract_string_literal(s, import_decl.source.start, import_decl.source.end);

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

		// Process dynamic imports
		for (const dynamic_import of parsed.module.dynamicImports) {
			// Find the corresponding AST node
			let found = false;
			for (const node of parsed.program.body) {
				if (
					node.type === 'ExpressionStatement' &&
					node.expression.type === 'AwaitExpression' &&
					node.expression.argument.type === 'ImportExpression'
				) {
					const import_expr = node.expression.argument;
					if (import_expr.start === dynamic_import.start) {
						// Only process string literals (not expressions or variables)
						if (import_expr.source.type === 'Literal') {
							const value = String(import_expr.source.value);
							if (value) {
								specifiers.push(value);
							}
						}
						found = true;
						break;
					}
				}
			}

			// If we didn't find a match through AST, fall back to the original approach
			// but only for simple string literals
			if (!found) {
				const value = extract_string_literal(
					s,
					dynamic_import.moduleRequest.start,
					dynamic_import.moduleRequest.end,
				);
				if (value) {
					specifiers.push(value);
				}
			}
		}

		// Process re-exports
		for (const node of parsed.program.body) {
			if (node.type === 'ExportNamedDeclaration' && node.source) {
				// Skip type-only exports if ignore_types is true
				if (ignore_types && node.exportKind === 'type') {
					continue;
				}

				if (node.source.type === 'Literal') {
					const value = node.source.value;
					if (value) {
						specifiers.push(value);
					}
				} else {
					throw new UnreachableError(node.source.type);
				}
			}
		}
	};

	if (is_svelte) {
		// Reset the regexp state between calls
		SVELTE_SCRIPT_MATCHER.lastIndex = 0;

		// Capture script tags at the top level (not nested in HTML)
		let last_index = 0;
		const script_blocks: Array<{content: string; start: number; end: number}> = [];

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
					end,
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
