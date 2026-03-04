import {describe, test, expect} from 'vitest';
import {dirname, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';

import {
	infer_declarations_from_file_type,
	process_ts_exports,
	type ExportDeclaration,
} from '../lib/parse_exports.ts';

import {create_ts_test_env} from './test_helpers.ts';

const dir = resolve(dirname(fileURLToPath(import.meta.url)), '../lib');

const create_declaration_map = (declarations: Array<ExportDeclaration>) =>
	Object.fromEntries(declarations.map((d) => [d.name, d.kind]));

describe('infer_declarations_from_file_type', () => {
	test('detects Svelte components', () => {
		expect(infer_declarations_from_file_type('Component.svelte')).toEqual([
			{name: 'default', kind: 'component'},
		]);
	});

	test('detects CSS files', () => {
		expect(infer_declarations_from_file_type('styles.css')).toEqual([
			{name: 'default', kind: 'css'},
		]);
	});

	test('detects JSON files', () => {
		expect(infer_declarations_from_file_type('data.json')).toEqual([
			{name: 'default', kind: 'json'},
		]);
	});

	test('does not infer for TypeScript files', () => {
		expect(infer_declarations_from_file_type('module.ts')).toEqual([]);
	});

	test('handles paths with directories', () => {
		expect(infer_declarations_from_file_type('src/components/Header.svelte')).toEqual([
			{name: 'default', kind: 'component'},
		]);
	});

	test('handles extensions case-insensitively', () => {
		expect(infer_declarations_from_file_type('Component.SVELTE')).toEqual([
			{name: 'default', kind: 'component'},
		]);
	});

	test('accumulates declarations', () => {
		expect(
			infer_declarations_from_file_type('Component.svelte', [{name: 'existing', kind: 'variable'}]),
		).toEqual([
			{name: 'existing', kind: 'variable'},
			{name: 'default', kind: 'component'},
		]);
	});
});

describe('process_ts_exports', () => {
	test('correctly identifies direct exports', () => {
		const source_code = `
			export const variable_export = 'test';
			export function function_export() { return true; }
			export class ClassExport {}
			export type TypeExport = string;
			export interface InterfaceExport {}
		`;

		const {source_file, program, exports: export_symbols} = create_ts_test_env(source_code, dir);

		const declarations = process_ts_exports(source_file, program, export_symbols);

		expect(create_declaration_map(declarations)).toEqual({
			variable_export: 'variable',
			function_export: 'function',
			ClassExport: 'class',
			TypeExport: 'type',
			InterfaceExport: 'type',
		});
	});

	test('correctly identifies named exports', () => {
		const source_code = `
			const variable_value = 'test';
			function function_value() { return true; }
			class ClassValue {}
			type TypeValue = string;
			interface InterfaceValue {}
			
			export { 
				variable_value, 
				function_value, 
				ClassValue,
				TypeValue,
				InterfaceValue
			};
		`;

		const {source_file, program, exports: export_symbols} = create_ts_test_env(source_code, dir);

		const declarations = process_ts_exports(source_file, program, export_symbols);

		expect(create_declaration_map(declarations)).toEqual({
			variable_value: 'variable',
			function_value: 'function',
			ClassValue: 'class',
			TypeValue: 'type',
			InterfaceValue: 'type',
		});
	});

	test('correctly identifies renamed exports', () => {
		const source_code = `
			const original_variable = 'test';
			function original_function() { return true; }
			class OriginalClass {}
			type OriginalType = string;
			
			export { 
				original_variable as renamed_variable, 
				original_function as renamed_function,
				OriginalClass as RenamedClass,
				type OriginalType as RenamedType
			};
		`;

		const {source_file, program, exports: export_symbols} = create_ts_test_env(source_code, dir);

		const declarations = process_ts_exports(source_file, program, export_symbols);

		expect(create_declaration_map(declarations)).toEqual({
			renamed_variable: 'variable',
			renamed_function: 'function',
			RenamedClass: 'class',
			RenamedType: 'type',
		});
	});

	test('correctly identifies type-only exports', () => {
		const source_code = `
			type RegularType = string;
			export type DirectType = number;
			
			interface RegularInterface { foo: string; }
			export interface DirectInterface { bar: number; }
			
			export type { RegularType, RegularInterface };
		`;

		const {source_file, program, exports: export_symbols} = create_ts_test_env(source_code, dir);

		const declarations = process_ts_exports(source_file, program, export_symbols);

		expect(create_declaration_map(declarations)).toEqual({
			DirectType: 'type',
			DirectInterface: 'type',
			RegularType: 'type',
			RegularInterface: 'type',
		});
	});

	test('correctly identifies function exports', () => {
		const source_code = `
			const arrow_function = () => 'arrow';
			const multi_line_arrow = () => {
				return 'multi-line arrow';
			};
			function declared_function() { return 'declared'; }
			
			export { arrow_function, multi_line_arrow, declared_function };
		`;

		const {source_file, program, exports: export_symbols} = create_ts_test_env(source_code, dir);

		const declarations = process_ts_exports(source_file, program, export_symbols);

		expect(create_declaration_map(declarations)).toEqual({
			arrow_function: 'function',
			multi_line_arrow: 'function',
			declared_function: 'function',
		});
	});

	test('correctly identifies class exports', () => {
		const source_code = `
			class SimpleClass {}
			
			const class_expression = class NamedClass { 
				method() {}
			};
			
			export { SimpleClass, class_expression };
		`;

		const {source_file, program, exports: export_symbols} = create_ts_test_env(source_code, dir);

		const declarations = process_ts_exports(source_file, program, export_symbols);

		expect(create_declaration_map(declarations)).toEqual({
			SimpleClass: 'class',
			class_expression: 'class',
		});
	});

	test('correctly identifies default exports', () => {
		const source_code = `
			function test_function() { return true; }
			export default test_function;
		`;

		const {source_file, program, exports: export_symbols} = create_ts_test_env(source_code, dir);

		const declarations = process_ts_exports(source_file, program, export_symbols);

		expect(create_declaration_map(declarations)).toEqual({
			default: 'function',
		});
	});

	test('correctly identifies inline default exports', () => {
		const source_code = `
			export default function() { return true; }
		`;

		const {source_file, program, exports: export_symbols} = create_ts_test_env(source_code, dir);

		const declarations = process_ts_exports(source_file, program, export_symbols);

		expect(create_declaration_map(declarations)).toEqual({
			default: 'function',
		});
	});

	test('correctly identifies dual purpose exports', () => {
		const source_code = `
			// Symbol is both a value and a type
			const dual_purpose = 'I am both value and type';
			type dual_purpose = string;
			
			// Export as a value
			export { dual_purpose };
			
			// Also export as a type
			export type { dual_purpose as dual_purpose_type };
		`;

		const {source_file, program, exports: export_symbols} = create_ts_test_env(source_code, dir);

		const declarations = process_ts_exports(source_file, program, export_symbols);

		expect(create_declaration_map(declarations)).toEqual({
			dual_purpose: 'variable', // Should be identified as variable when exported as value
			dual_purpose_type: 'type', // And as type when exported as type
		});
	});

	test('correctly handles type-based exports of dual purpose symbols', () => {
		const source_code = `
			// Symbol is both a value and a type
			const dual_purpose = 'I am both value and type';
			type dual_purpose = string;
			
			// Only export as a type, not as a value
			export type { dual_purpose };
		`;

		const {source_file, program, exports: export_symbols} = create_ts_test_env(source_code, dir);

		const declarations = process_ts_exports(source_file, program, export_symbols);

		expect(create_declaration_map(declarations)).toEqual({
			dual_purpose: 'type', // Should be identified as type when only exported as type
		});
	});

	test('correctly handles aliased dual purpose exports', () => {
		const source_code = `
			// Symbol is both a value and a type
			const dual_purpose = 'I am both value and type';
			type dual_purpose = string;
			
			// Export as a value with an alias
			export { dual_purpose as dual_purpose_alias };
			
			// Also export as a type with a different alias
			export type { dual_purpose as dual_purpose_type_alias };
		`;

		const {source_file, program, exports: export_symbols} = create_ts_test_env(source_code, dir);

		const declarations = process_ts_exports(source_file, program, export_symbols);

		expect(create_declaration_map(declarations)).toEqual({
			dual_purpose_alias: 'variable', // Should be identified as variable when exported as value
			dual_purpose_type_alias: 'type', // Should be identified as type when exported as type
		});
	});

	test('correctly identifies re-exported functions', () => {
		const module_a = `
			export function example_function() { return true; }
		`;

		const module_b = `
			export {example_function} from './module_a.ts';
			export {example_function as renamed_function} from './module_a.ts';
		`;

		const {
			source_file: module_b_source,
			program,
			exports: export_symbols,
		} = create_ts_test_env(module_b, dir, {
			'./module_a.ts': module_a,
		});

		const declarations = process_ts_exports(module_b_source, program, export_symbols);

		expect(create_declaration_map(declarations)).toEqual({
			example_function: 'function',
			renamed_function: 'function',
		});
	});

	test('correctly identifies re-exported functions with type exports', () => {
		const plugin_module = `
			export interface Plugin {
				name: string;
			}

			export const plugin_replace = (
				plugins,
				new_plugin,
				name = new_plugin.name,
			) => {
				return [new_plugin];
			};
		`;

		const index_module = `
			export {type Plugin, plugin_replace} from './plugin_module.ts';
		`;

		const {
			source_file: index_source,
			program,
			exports: export_symbols,
		} = create_ts_test_env(index_module, dir, {
			'./plugin_module.ts': plugin_module,
		});

		const declarations = process_ts_exports(index_source, program, export_symbols);

		expect(create_declaration_map(declarations)).toEqual({
			Plugin: 'type',
			plugin_replace: 'function',
		});
	});

	test('correctly identifies plugin_replace from actual plugin.ts', () => {
		// Create a simplified version of the index.ts content
		const index_code = `
			export {type Plugin, plugin_replace} from './plugin.ts';
		`;

		// Create the test environment with the actual plugin.ts file
		const {source_file, program, exports: export_symbols} = create_ts_test_env(index_code, dir);

		const declarations = process_ts_exports(source_file, program, export_symbols);

		// Find the plugin_replace export
		const plugin_replace_declaration = declarations.find((d) => d.name === 'plugin_replace');
		expect(plugin_replace_declaration).toBeTruthy();
		expect(plugin_replace_declaration?.kind).toBe('function');
	});
});
