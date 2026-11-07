import { describe, test, assert } from 'vitest';

import { parse_imports, type ImportSpecifier } from '#lib/internal/parse_imports.ts';

const parse_specifiers = (id: string, contents: string): Array<ImportSpecifier> => {
	const parsed = parse_imports(id, contents);
	assert.ok(parsed.ok, parsed.ok ? undefined : parsed.error.message);
	return parsed.specifiers;
};

describe('parse_imports', () => {
	describe('forms', () => {
		test('static imports of every binding shape', () => {
			const source = `
				import default_import from './default.ts';
				import * as namespace_import from './namespace.ts';
				import default_and_named, {named} from './default_and_named.ts';
				import {a as b} from './renamed.ts';
				import {} from './empty_list.ts';
				import './side_effect.ts';
			`;
			assert.deepEqual(parse_specifiers('a.ts', source), [
				'./default.ts',
				'./namespace.ts',
				'./default_and_named.ts',
				'./renamed.ts',
				'./empty_list.ts',
				'./side_effect.ts'
			]);
		});

		test('re-exports, named and export-all', () => {
			const source = `
				export {a} from './named.ts';
				export * from './star.ts';
				export * as ns from './ns.ts';
				export {default} from './default.ts';
			`;
			assert.deepEqual(parse_specifiers('a.ts', source), [
				'./named.ts',
				'./star.ts',
				'./ns.ts',
				'./default.ts'
			]);
		});

		test('dynamic imports anywhere, single strings only', () => {
			const source = `
				const a = import('./single.ts');
				const b = import("./double.ts");
				const c = import(\`./template.ts\`);
				const d = () => import('./in_arrow.ts');
				if (x) await import('./in_block.ts');
				const e = import(\`./locales/\${lang}.ts\`);
				const f = import('./x' + 'y');
				const g = import(\`./x\` + \`y\`);
				const h = import(dynamic_variable);
			`;
			assert.deepEqual(parse_specifiers('a.ts', source), [
				'./single.ts',
				'./double.ts',
				'./template.ts',
				'./in_arrow.ts',
				'./in_block.ts'
			]);
		});

		test('import-equals requires', () => {
			const source = `
				import a = require('./value.ts');
				export import b = require('./exported.ts');
			`;
			assert.deepEqual(parse_specifiers('a.ts', source), ['./value.ts', './exported.ts']);
		});

		test('imports with attributes', () => {
			const source = `
				import data from './data.json' with {type: 'json'};
				const lazy = await import('./lazy.json', {with: {type: 'json'}});
			`;
			assert.deepEqual(parse_specifiers('a.ts', source), ['./data.json', './lazy.json']);
		});

		test('source-phase and deferred imports', () => {
			const source = `
				import defer * as deferred from './deferred.ts';
				import source wasm from './module.wasm';
				import.defer('./dynamic_deferred.ts');
				import.source('./dynamic_module.wasm');
			`;
			assert.deepEqual(parse_specifiers('a.ts', source), [
				'./deferred.ts',
				'./module.wasm',
				'./dynamic_deferred.ts',
				'./dynamic_module.wasm'
			]);
		});

		test('in source order across forms', () => {
			const source = `
				export * from './re_export.ts';
				const a = await import('./dynamic.ts');
				import './static.ts';
				import b = require('./required.ts');
			`;
			assert.deepEqual(parse_specifiers('a.ts', source), [
				'./re_export.ts',
				'./dynamic.ts',
				'./static.ts',
				'./required.ts'
			]);
		});

		test('duplicates kept', () => {
			const source = `
				import {a} from './a.ts';
				import {b} from './a.ts';
				const c = await import('./a.ts');
			`;
			assert.deepEqual(parse_specifiers('a.ts', source), ['./a.ts', './a.ts', './a.ts']);
		});

		test('empty specifiers skipped', () => {
			const source = `
				import {foo} from '';
				await import('');
				await import(\`\`);
			`;
			assert.deepEqual(parse_specifiers('a.ts', source), []);
		});
	});

	describe('types', () => {
		test('type-only statements skipped', () => {
			const source = `
				import type {A} from './type_named.ts';
				import type B from './type_default.ts';
				import type * as C from './type_namespace.ts';
				export type {D} from './type_re_export.ts';
				export type * from './type_star.ts';
				export type * as E from './type_star_ns.ts';
				import type F = require('./type_required.ts');
				import {g} from './value.ts';
			`;
			assert.deepEqual(parse_specifiers('a.ts', source), ['./value.ts']);
		});

		test('skipped only when every inline specifier is a type', () => {
			const source = `
				import {type a, type b} from './all_inline_types.ts';
				import {type c, d} from './mixed.ts';
				import e, {type f} from './default_and_type.ts';
				export {type G} from './all_inline_type_re_export.ts';
				export {type H, i} from './mixed_re_export.ts';
			`;
			assert.deepEqual(parse_specifiers('a.ts', source), [
				'./mixed.ts',
				'./default_and_type.ts',
				'./mixed_re_export.ts'
			]);
		});

		test('type positions hold none', () => {
			const source = `
				type A = typeof import('./type_query.ts');
				type B = import('./type_ref.ts').B;
				let c: Array<import('./type_nested.ts').C>;
				const d = await import('./dynamic.ts');
			`;
			assert.deepEqual(parse_specifiers('a.ts', source), ['./dynamic.ts']);
		});

		test('ambient declarations hold none', () => {
			const source = `
				declare module 'ambient' {
					import x from './ambient.ts';
				}
				import './value.ts';
			`;
			assert.deepEqual(parse_specifiers('a.ts', source), ['./value.ts']);
		});

		test('around TypeScript syntax and comments', () => {
			const source = `
				const a: 5 = 5;
				interface B {
					b: 456;
				}
				import {
					// a comment
					foo,
					/* a block
					   comment */
					bar,
				} from './with_comments.ts';
				const c = f<number>(1);
			`;
			assert.deepEqual(parse_specifiers('a.ts', source), ['./with_comments.ts']);
		});
	});

	describe('decoding', () => {
		test('static specifiers decoded, escapes included', () => {
			const source = `
				import a from './a\\'b.ts';
				import {c} from "./c\\\\d.ts";
				import './\\x65.ts';
				import "./it's.ts";
			`;
			assert.deepEqual(parse_specifiers('a.ts', source), [
				"./a'b.ts",
				'./c\\d.ts',
				'./e.ts',
				"./it's.ts"
			]);
		});

		test('dynamic specifiers decoded, escapes included', () => {
			const source = `
				const a = await import('./a\\'b.ts');
				const f = () => import("./c\\\\d.ts");
				if (x) await import(\`./\\x65.ts\`);
			`;
			assert.deepEqual(parse_specifiers('a.ts', source), ["./a'b.ts", './c\\d.ts', './e.ts']);
		});
	});

	describe('languages', () => {
		test('JS and TS by every extension', () => {
			const source = `import './x.ts';`;
			for (const id of ['a.js', 'a.mjs', 'a.cjs', 'a.ts', 'a.mts', 'a.cts', 'a.svelte.ts']) {
				assert.deepEqual(parse_specifiers(id, source), ['./x.ts'], id);
			}
		});

		test('a script, not a module, retried in sloppy mode', () => {
			const source = `
				with (o) { x; }
				import('./sloppy.ts');
			`;
			assert.deepEqual(parse_specifiers('a.js', source), ['./sloppy.ts']);
		});

		test('other files have none', () => {
			assert.deepEqual(parse_specifiers('a.css', `@import './b.css';`), []);
			assert.deepEqual(parse_specifiers('a.json', `{"import": "./x.ts"}`), []);
			assert.deepEqual(parse_specifiers('a.md', `import './x.ts';`), []);
		});
	});

	describe('svelte', () => {
		test('both scripts in source order, TS and JS', () => {
			const source = `
				<script lang="ts" module>
					const a: 5 = 5;
					import type {T} from './module_types.ts';
					import {foo} from './module_static.ts';
					await import('./module_dynamic.ts');
				</script>

				<p>between</p>

				<script>
					import {bar} from './instance_static.ts';
					await import('./instance_dynamic.ts');
				</script>
			`;
			assert.deepEqual(parse_specifiers('a.svelte', source), [
				'./module_static.ts',
				'./module_dynamic.ts',
				'./instance_static.ts',
				'./instance_dynamic.ts'
			]);
		});

		test('the instance script first when written first', () => {
			const source = `
				<script>import './instance.ts';</script>
				<script module>import './module.ts';</script>
			`;
			assert.deepEqual(parse_specifiers('a.svelte', source), ['./instance.ts', './module.ts']);
		});

		test('the legacy module context attribute', () => {
			const source = `<script context="module">import './module.ts';</script>`;
			assert.deepEqual(parse_specifiers('a.svelte', source), ['./module.ts']);
		});

		test('a script element nested in markup holds none', () => {
			const source = `
				<script>import './instance.ts';</script>
				<div>
					<pre>
						<script>
							import {not_real} from './nested.ts';
						</script>
					</pre>
				</div>
			`;
			assert.deepEqual(parse_specifiers('a.svelte', source), ['./instance.ts']);
		});

		test('markup expressions and styles hold none', () => {
			// TODO a dynamic import in markup is a dependency, see the TODO in `parse_imports`
			const source = `
				<script>import './instance.ts';</script>
				<p>{import('./markup.ts')}</p>
				<style>@import './style.css';</style>
			`;
			assert.deepEqual(parse_specifiers('a.svelte', source), ['./instance.ts']);
		});

		test('a leading byte-order mark', () => {
			assert.deepEqual(parse_specifiers('a.svelte', `\uFEFF<script>import './a.ts';</script>`), [
				'./a.ts'
			]);
		});

		test('no scripts has none', () => {
			assert.deepEqual(parse_specifiers('a.svelte', `<p>hi</p>`), []);
		});
	});

	describe('errors', () => {
		test('TS that does not parse returns the error and its location', () => {
			const parsed = parse_imports('a.ts', `import './ok.ts';\nimport {a from './a.ts';`);
			assert.ok(!parsed.ok);
			assert.equal(parsed.error.kind, 'parse');
			assert.include(parsed.error.message, 'from');
			assert.equal(parsed.error.loc.start.line, 2);
		});

		test('Svelte script that does not parse returns the error', () => {
			const parsed = parse_imports('a.svelte', `<script>import {a from './a.ts';</script>`);
			assert.ok(!parsed.ok);
			assert.equal(parsed.error.kind, 'parse');
		});

		test('Svelte markup that does not parse returns the error', () => {
			const parsed = parse_imports('a.svelte', `<script>import './a.ts';</script>\n<div>`);
			assert.ok(!parsed.ok);
			assert.include(parsed.error.message, '<div>');
		});
	});
});
