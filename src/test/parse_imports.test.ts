import { test, expect } from 'vitest';

import { parse_imports } from '$lib/parse_imports.ts';

test('parse js imports', () => {
	const parsed = parse_imports(
		'a.js',
		`
      const a = 5;
      import {foo} from 'static_import';
      await import('dynamic_import');
      const b = {};
    `
	);
	expect(parsed).toEqual(['static_import', 'dynamic_import']);
});

test('parse ts imports', () => {
	const parsed = parse_imports(
		'a.ts',
		`
      const a: 5 = 5;
      type A = 123;
      interface B {
        b: 456;
      }
      import {foo} from 'static_import';
      await import('dynamic_import');
      const b: {} = {};
      export type {G} from 'exported_import';
    `
	);
	expect(parsed).toEqual(['static_import', 'dynamic_import']);
});

test('parse ts imports and omit types', () => {
	const parsed = parse_imports(
		'a.ts',
		`
      import type {foo} from 'static_import';
      import {type bar} from 'inline_type_import';
      import {type baz, qux} from 'mixed_type_import';
      import {aaa, type bbb, ccc} from 'mixed_type_import2';
      await import('dynamic_import');
    `
	);
	expect(parsed).toEqual(['mixed_type_import', 'mixed_type_import2', 'dynamic_import']);
});

test('parse ts imports and include types', () => {
	const parsed = parse_imports(
		'a.ts',
		`
      import type {foo} from 'static_import';
      import {type bar} from 'inline_type_import';
      import {type baz, qux} from 'mixed_type_import';
      import {aaa, type bbb, ccc} from 'mixed_type_import2';
      await import('dynamic_import');
      export type {G} from 'exported_import';
    `,
		false
	);
	expect(parsed).toEqual([
		'static_import',
		'inline_type_import',
		'mixed_type_import',
		'mixed_type_import2',
		'dynamic_import',
		'exported_import'
	]);
});

test('parse svelte imports', () => {
	const parsed = parse_imports(
		'a.svelte',
		`
      000

      <!-- TS module script -->
      <script lang="ts" module>
        const a2: 5 = 5;
        import {foo2} from 'static_import_module_context';
        await import('dynamic_import_module_context');
        const b2: {} = {};
      </script>
      
      111
      
      <!-- TS script -->
      <script lang="ts">
        const a: 5 = 5;
        import {foo} from 'static_import';
        await import('dynamic_import');
        const b: {} = {};
      </script>
      
      222
    `
	);
	expect(parsed).toEqual([
		'static_import_module_context',
		'dynamic_import_module_context',
		'static_import',
		'dynamic_import'
	]);
});

test('parse plain JS svelte imports', () => {
	const parsed = parse_imports(
		'a.svelte',
		`
      <script>
        const a = 5;
        import {foo3} from 'static_import';
        await import('dynamic_import');
        const b3 = {};
      </script>
    `
	);
	expect(parsed).toEqual(['static_import', 'dynamic_import']);
});

test('parse empty imports', () => {
	const parsed = parse_imports(
		'a.ts',
		`
      const a: 5 = 5;
      type A = 123;
      interface B {
        b: 456;
      }
      import {foo} from '';
      await import('');
      const b: {} = {};
    `
	);
	expect(parsed).toEqual([]);
});

test('parse ts re-exports with type keywords', () => {
	const parsed = parse_imports(
		'a.ts',
		`
      export {something} from 'value_export';
      export type {TypeA} from 'type_export';
      export {type InlineType, ValueExport} from 'mixed_export';
    `
	);
	expect(parsed).toEqual(['value_export', 'mixed_export']);
});

test('parse ts re-exports with type keywords (include types)', () => {
	const parsed = parse_imports(
		'a.ts',
		`
      export {something} from 'value_export';
      export type {TypeA} from 'type_export';
      export {type InlineType, ValueExport} from 'mixed_export';
    `,
		false
	);
	expect(parsed).toEqual(['value_export', 'type_export', 'mixed_export']);
});

test('parse default and namespace imports', () => {
	const parsed = parse_imports(
		'a.ts',
		`
      import defaultImport from 'default_module';
      import * as namespaceImport from 'namespace_module';
      import defaultAndNamed, {named} from 'mixed_default_named';
      import type DefaultType from 'default_type_module';
      import type * as NamespaceType from 'namespace_type_module';
    `
	);
	expect(parsed).toEqual(['default_module', 'namespace_module', 'mixed_default_named']);
});

test('parse imports with comments and whitespace', () => {
	const parsed = parse_imports(
		'a.ts',
		`
      import {
        // This is a comment
        foo,
        /* Multi
           line
           comment */
        bar,
      } from 'module_with_comments';
      
      import type {
        CommentedType, // End line comment
      } from 'type_with_comments';
    `
	);
	expect(parsed).toEqual(['module_with_comments']);
});

test('parse imports in complex svelte files', () => {
	const parsed = parse_imports(
		'a.svelte',
		`
      <script context="module" lang="ts">
        import type {SomeType} from 'module_types';
        import {helper} from 'module_helpers';
      </script>
      
      <script lang="ts">
        import {Component} from './component';
        import type {Props} from './types';
      </script>
      
      <!-- Nested script tags in HTML shouldn't be parsed -->
      <div>
        <pre>
          <script>
            import {shouldNotBeParsed} from 'not_real';
          </script>
        </pre>
      </div>
    `
	);
	expect(parsed).toEqual(['module_helpers', './component']);
});

test('parse imports with string literals in different formats', () => {
	const parsed = parse_imports(
		'a.ts',
		`
      import {a} from "double_quotes";
      import {b} from 'single_quotes';
    `
	);
	expect(parsed).toEqual(['double_quotes', 'single_quotes']);
});

test('parse dynamic imports with expressions', () => {
	const parsed = parse_imports(
		'a.ts',
		`
      await import('simple_dynamic');
      await import(\`template_\${dynamic}\`);
      await import(dynamicVariable);
      await import('./path/' + moduleName);
    `
	);
	// Only the string literal should be captured
	expect(parsed).toEqual(['simple_dynamic']);
});
