import ts from 'typescript';
import type {Logger} from '@fuzdev/fuz_util/log.js';
import type {DeclarationKind} from '@fuzdev/fuz_util/source_json.js';

import type {ExportDeclaration} from './parse_exports.ts';

/**
 * A class to track export context and determine export kinds.
 */
export class ParseExportsContext {
	readonly #checker: ts.TypeChecker;

	// Map of source file paths to their symbols
	readonly #file_symbols: Map<string, ts.Symbol> = new Map();
	// Cache for resolved symbols to avoid repeated resolution
	readonly #symbol_kind_cache: Map<ts.Symbol, DeclarationKind> = new Map();

	readonly log: Logger | undefined;
	debug = process.env.DEBUG_EXPORTS === 'true';

	constructor(program: ts.Program, log?: Logger) {
		this.log = log;
		this.#checker = program.getTypeChecker();
	}

	/**
	 * Log a debug message if debug mode is enabled.
	 */
	#log(...args: Array<unknown>): void {
		if (this.debug && this.log) {
			this.log.info(...args);
		}
	}

	/**
	 * Analyze a source file to prepare for export processing.
	 */
	analyze_source_file(source_file: ts.SourceFile): void {
		const file_path = source_file.fileName;

		// Skip if we've already analyzed this file
		if (this.#file_symbols.has(file_path)) {
			return;
		}

		// Get the source file symbol and cache it
		const symbol = this.#checker.getSymbolAtLocation(source_file);
		if (symbol) {
			this.#file_symbols.set(file_path, symbol);
		}
	}

	/**
	 * Process a list of exported symbols and identify their kinds.
	 */
	process_exports(
		source_file: ts.SourceFile,
		exports: Array<ts.Symbol>,
		declarations: Array<ExportDeclaration> = [],
	): Array<ExportDeclaration> {
		for (const export_symbol of exports) {
			const name = export_symbol.name;
			this.#log(`Determining kind for export: ${name}`);

			const kind = this.#determine_export_kind(source_file, export_symbol);
			declarations.push({
				name,
				kind,
			});
		}

		return declarations;
	}

	/**
	 * Determine the kind of an export based on its symbol.
	 */
	#determine_export_kind(source_file: ts.SourceFile, symbol: ts.Symbol): DeclarationKind {
		// Check if this is a type-only export (no value export)
		if (this.#is_type_only_export(source_file, symbol)) {
			return 'type';
		}

		// Get the true symbol by resolving aliases
		const resolved_symbol = this.#resolve_symbol(symbol);

		// Check if we've already determined this symbol's kind
		if (this.#symbol_kind_cache.has(resolved_symbol)) {
			return this.#symbol_kind_cache.get(resolved_symbol)!;
		}

		// Determine the kind based on declaration and type information
		const kind = this.#infer_declaration_kind(resolved_symbol);

		// Cache the result for future lookups
		this.#symbol_kind_cache.set(resolved_symbol, kind);

		return kind;
	}

	/**
	 * Resolve a symbol through aliases to its original declaration.
	 */
	#resolve_symbol(symbol: ts.Symbol): ts.Symbol {
		try {
			if (symbol.flags & ts.SymbolFlags.Alias) {
				return this.#checker.getAliasedSymbol(symbol);
			}
		} catch {
			// If resolution fails, return the original symbol
		}
		return symbol;
	}

	/**
	 * Infer the declaration kind from a symbol's declaration and type information.
	 */
	#infer_declaration_kind(symbol: ts.Symbol): DeclarationKind {
		// Check symbol flags first for direct type matching
		if (this.#is_class_symbol(symbol)) {
			return 'class';
		}

		if (this.#is_function_symbol(symbol)) {
			return 'function';
		}

		// If no direct match from flags, look at declarations
		if (symbol.declarations && symbol.declarations.length > 0) {
			const decl = symbol.declarations[0]!;
			const kind_from_decl = this.#infer_kind_from_declaration(decl);
			if (kind_from_decl) {
				return kind_from_decl;
			}
		}

		// Check for callable type as a fallback for functions
		if (this.#is_callable(symbol)) {
			return 'function';
		}

		// Default to variable if no other type can be determined
		return 'variable';
	}

	/**
	 * Check if a symbol represents a callable type (function-like).
	 */
	#is_callable(symbol: ts.Symbol): boolean {
		try {
			// Try to get valid declaration node
			const declaration = this.#get_valid_declaration(symbol);
			if (!declaration) {
				return false;
			}

			// Get the type at the declaration location
			const type = this.#checker.getTypeOfSymbolAtLocation(symbol, declaration);

			// Check if the type has call signatures (making it function-like)
			return type.getCallSignatures().length > 0;
		} catch {
			return false;
		}
	}

	/**
	 * Get a valid declaration for a symbol, preferring valueDeclaration.
	 */
	#get_valid_declaration(symbol: ts.Symbol): ts.Node | undefined {
		if (symbol.valueDeclaration) {
			return symbol.valueDeclaration;
		}

		if (symbol.declarations && symbol.declarations.length > 0) {
			return symbol.declarations[0];
		}

		return undefined;
	}

	/**
	 * Infer the declaration kind from a specific declaration node.
	 */
	#infer_kind_from_declaration(decl: ts.Declaration): DeclarationKind | null {
		if (ts.isFunctionDeclaration(decl)) {
			return 'function';
		}

		if (ts.isClassDeclaration(decl)) {
			return 'class';
		}

		if (ts.isInterfaceDeclaration(decl) || ts.isTypeAliasDeclaration(decl)) {
			return 'type';
		}

		if (ts.isVariableDeclaration(decl)) {
			// Handle initializers for variable declarations
			if (decl.initializer) {
				if (ts.isFunctionExpression(decl.initializer) || ts.isArrowFunction(decl.initializer)) {
					return 'function';
				}

				if (ts.isClassExpression(decl.initializer)) {
					return 'class';
				}

				// Handle identifiers pointing to other declarations
				if (ts.isIdentifier(decl.initializer)) {
					try {
						const referred_symbol = this.#checker.getSymbolAtLocation(decl.initializer);
						if (referred_symbol) {
							// Avoid infinite recursion by not resolving symbols here
							if (this.#is_function_symbol(referred_symbol)) {
								return 'function';
							}
							if (this.#is_class_symbol(referred_symbol)) {
								return 'class';
							}
						}
					} catch {
						// Ignore failures to resolve identifiers
					}
				}
			}

			// As a fallback, check if the variable's type is callable
			try {
				const symbol = this.#checker.getSymbolAtLocation(decl.name);
				if (symbol && this.#is_callable(symbol)) {
					return 'function';
				}
			} catch {
				// Ignore errors in type checking
			}
		}

		return null;
	}

	/**
	 * Check if a symbol is exported as a type-only export.
	 * A type-only export means it's ONLY exported as a type with no value export.
	 */
	#is_type_only_export(source_file: ts.SourceFile, symbol: ts.Symbol): boolean {
		// First, check if the symbol has an explicit type-only export
		let has_type_only_export = false;

		// Check if it has a corresponding value export
		const has_value_export = this.#has_value_export(source_file, symbol);

		// If it has both type and value exports (dual purpose), it's not type-only
		if (has_value_export) {
			return false;
		}

		// Check export declarations for explicit type-only exports
		ts.forEachChild(source_file, (node) => {
			if (
				ts.isExportDeclaration(node) &&
				node.exportClause &&
				ts.isNamedExports(node.exportClause)
			) {
				// Check if it's a type-only export declaration (export type {...})
				if (node.isTypeOnly) {
					for (const specifier of node.exportClause.elements) {
						if (specifier.name.text === symbol.name) {
							has_type_only_export = true;
						}
					}
				} else {
					// Check if it's a specific type export (export {type X})
					for (const specifier of node.exportClause.elements) {
						if (specifier.name.text === symbol.name && specifier.isTypeOnly) {
							has_type_only_export = true;
						}
					}
				}
			}
		});

		// If explicitly marked as a type-only export, use that

		if (has_type_only_export) {
			return true;
		}

		// If not explicitly marked as type-only, check if the symbol itself is a type
		// AND there's no value export for it
		const resolved_symbol = this.#resolve_symbol(symbol);
		return this.#is_type_symbol(resolved_symbol) && !has_value_export;
	}

	/**
	 * Check if a symbol has a value export in the source file.
	 */
	#has_value_export(source_file: ts.SourceFile, symbol: ts.Symbol): boolean {
		let has_value_export = false;

		// Check export declarations
		ts.forEachChild(source_file, (node) => {
			if (
				ts.isExportDeclaration(node) &&
				node.exportClause &&
				ts.isNamedExports(node.exportClause)
			) {
				// Skip type-only exports
				if (node.isTypeOnly) return;

				// Check if it's a regular export (not type-only)
				for (const specifier of node.exportClause.elements) {
					if (specifier.name.text === symbol.name && !specifier.isTypeOnly) {
						has_value_export = true;
					}
				}
			}
			// Check for default export
			else if (ts.isExportAssignment(node) && symbol.name === 'default') {
				has_value_export = true;
			}
			// Check for direct exports (export const x = ...)
			else if (this.#is_direct_export(node) && this.#get_export_name(node) === symbol.name) {
				has_value_export = true;
			}
		});

		return has_value_export;
	}

	/**
	 * Check if a node is a direct export (export const/function/class).
	 */
	#is_direct_export(node: ts.Node): boolean {
		return (
			(ts.isVariableStatement(node) ||
				ts.isFunctionDeclaration(node) ||
				ts.isClassDeclaration(node)) &&
			this.#has_export_modifier(node)
		);
	}

	/**
	 * Get the export name from a direct export node.
	 */
	#get_export_name(node: ts.Node): string | undefined {
		if (ts.isVariableStatement(node) && node.declarationList.declarations.length > 0) {
			const decl = node.declarationList.declarations[0]!;
			if (ts.isIdentifier(decl.name)) {
				return decl.name.text;
			}
		} else if (ts.isFunctionDeclaration(node) && node.name) {
			return node.name.text;
		} else if (ts.isClassDeclaration(node) && node.name) {
			return node.name.text;
		}
		return undefined;
	}

	/**
	 * Check if a node has an export modifier.
	 */
	#has_export_modifier(node: ts.Node): boolean {
		return (
			(ts.canHaveModifiers(node) &&
				ts.getModifiers(node)?.some((mod) => mod.kind === ts.SyntaxKind.ExportKeyword)) ||
			false
		);
	}

	/**
	 * Check if a symbol is a function symbol.
	 */
	#is_function_symbol(symbol: ts.Symbol): boolean {
		return !!(symbol.flags & ts.SymbolFlags.Function || symbol.flags & ts.SymbolFlags.Method);
	}

	/**
	 * Check if a symbol is a class symbol.
	 */
	#is_class_symbol(symbol: ts.Symbol): boolean {
		return !!(symbol.flags & ts.SymbolFlags.Class);
	}

	/**
	 * Check if a symbol is a type-only symbol (interface, type alias, etc.).
	 */
	#is_type_symbol(symbol: ts.Symbol): boolean {
		return !!(
			symbol.flags & ts.SymbolFlags.Interface ||
			symbol.flags & ts.SymbolFlags.TypeAlias ||
			symbol.flags & ts.SymbolFlags.TypeParameter
		);
	}
}
