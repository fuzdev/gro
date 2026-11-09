import { describe, test, assert, beforeEach, afterEach, vi } from 'vitest';
import { mkdtemp, mkdir, writeFile, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import {
	rewrite_relative_ts_imports,
	rewrite_dist_file_imports,
	rewrite_dist_imports
} from '#lib/dist_rewrite_imports.ts';
import { fs } from '#lib/fs.ts';
import { create_fs_paths } from '#lib/fs_paths.ts';

import { create_mock_logger } from './test_helpers.ts';

const fs_paths = create_fs_paths(fs);

const rewrite_svelte = (content: string): string =>
	rewrite_dist_file_imports('Comp.svelte', content);

const rewrite_js = (content: string): string => rewrite_dist_file_imports('a.js', content);

describe('rewrite_relative_ts_imports', () => {
	test('rewrites relative `.ts` specifiers to `.js`', () => {
		assert.equal(
			rewrite_relative_ts_imports(`import {a} from './a.ts';`),
			`import {a} from './a.js';`
		);
		assert.equal(
			rewrite_relative_ts_imports(`import {a} from '../parent/a.ts';`),
			`import {a} from '../parent/a.js';`
		);
	});

	test('rewrites the `.svelte.ts` double extension to `.svelte.js`', () => {
		assert.equal(
			rewrite_relative_ts_imports(`import {s} from './state.svelte.ts';`),
			`import {s} from './state.svelte.js';`
		);
	});

	test('leaves bare `@fuzdev/…ts` specifiers alone (the exports mirror resolves them)', () => {
		const input = `import {x} from '@fuzdev/fuz_util/string.ts';`;
		assert.equal(rewrite_relative_ts_imports(input), input);
	});

	test('leaves other bare specifiers alone', () => {
		const input = `import {page} from '$app/state';\nimport type {Snippet} from 'svelte';`;
		assert.equal(rewrite_relative_ts_imports(input), input);
	});

	test('leaves `.svelte` component imports alone', () => {
		const input = `import Svg from './Svg.svelte';`;
		assert.equal(rewrite_relative_ts_imports(input), input);
	});

	test('leaves specifiers already ending in `.js` alone', () => {
		const input = `import {a} from './a.js';\nimport {b} from './b.svelte.js';`;
		assert.equal(rewrite_relative_ts_imports(input), input);
	});

	test('rewrites `import type` specifiers', () => {
		assert.equal(
			rewrite_relative_ts_imports(`import type {Logger} from './log.ts';`),
			`import type {Logger} from './log.js';`
		);
	});

	test('rewrites `export … from` and `export * from` specifiers', () => {
		assert.equal(
			rewrite_relative_ts_imports(`export {a} from './a.ts';`),
			`export {a} from './a.js';`
		);
		assert.equal(rewrite_relative_ts_imports(`export * from './a.ts';`), `export * from './a.js';`);
	});

	test('rewrites side-effect imports', () => {
		assert.equal(rewrite_relative_ts_imports(`import './register.ts';`), `import './register.js';`);
	});

	test('rewrites dynamic and `.d.ts` `import(...)` type specifiers', () => {
		assert.equal(
			rewrite_relative_ts_imports(`const m = await import('./lazy.ts');`),
			`const m = await import('./lazy.js');`
		);
		assert.equal(
			rewrite_relative_ts_imports(`export declare const x: import("./types.ts").Foo;`),
			`export declare const x: import("./types.js").Foo;`
		);
	});

	test('handles both quote styles', () => {
		assert.equal(
			rewrite_relative_ts_imports(`import {a} from "./a.ts";`),
			`import {a} from "./a.js";`
		);
	});

	test('does not match `.tsx`, `.mts`, or query-suffixed specifiers', () => {
		const input = `import X from './X.tsx';\nimport {m} from './m.mts';\nimport raw from './r.ts?raw';`;
		assert.equal(rewrite_relative_ts_imports(input), input);
	});

	test("does not match member calls like `arr.from('./x.ts')`", () => {
		const input = `const a = Array.from('./x.ts');\nconst b = q.from('./y.ts');`;
		assert.equal(rewrite_relative_ts_imports(input), input);
	});

	test('rewrites every specifier across a realistic `.d.ts` body', () => {
		const input = [
			`import { type ContextmenuRootBaseProps } from './contextmenu_helpers.ts';`,
			`import type {SvgData} from './svg.ts';`,
			`import {Tome} from './tome.ts';`,
			`import {strip_start} from '@fuzdev/fuz_util/string.ts';`,
			`export * from './reexport.ts';`
		].join('\n');
		const expected = [
			`import { type ContextmenuRootBaseProps } from './contextmenu_helpers.js';`,
			`import type {SvgData} from './svg.js';`,
			`import {Tome} from './tome.js';`,
			`import {strip_start} from '@fuzdev/fuz_util/string.ts';`,
			`export * from './reexport.js';`
		].join('\n');
		assert.equal(rewrite_relative_ts_imports(input), expected);
	});
});

describe('rewrite_dist_file_imports in `.js`', () => {
	const REWRITTEN: Array<[string, string]> = [
		[`import {a} from './a.ts';`, `import {a} from './a.js';`],
		[`import {a} from '../parent/a.ts';`, `import {a} from '../parent/a.js';`],
		[`import {s} from './state.svelte.ts';`, `import {s} from './state.svelte.js';`],
		[`import {a} from "./a.ts";`, `import {a} from "./a.js";`],
		[`export {a} from './a.ts';`, `export {a} from './a.js';`],
		[`export * from './a.ts';`, `export * from './a.js';`],
		[`import './register.ts';`, `import './register.js';`],
		[`const m = await import('./lazy.ts');`, `const m = await import('./lazy.js');`]
	];
	for (const [input, expected] of REWRITTEN) {
		test(`rewrites ${input}`, () => {
			assert.equal(rewrite_js(input), expected);
		});
	}

	const UNCHANGED = [
		`import {x} from '@fuzdev/fuz_util/string.ts';`,
		`import Svg from './Svg.svelte';`,
		`import {a} from './a.js';`,
		`import X from './X.tsx';`,
		`import {m} from './m.mts';`,
		`import raw from './r.ts?raw';`,
		`const a = Array.from('./x.ts');`
	];
	for (const input of UNCHANGED) {
		test(`leaves ${input} alone`, () => {
			assert.equal(rewrite_js(input), input);
		});
	}

	test('leaves an import statement inside a template literal as written', () => {
		const input = "export const t = `import {x} from './feed.ts';`;\n";
		assert.equal(rewrite_js(input), input);
	});

	test('leaves an import statement inside a comment as written', () => {
		const input = `// import {x} from './line.ts';\n/* export * from './block.ts'; */\nexport const a = 1;\n`;
		assert.equal(rewrite_js(input), input);
	});

	test('rewrites the real imports around template literals and comments', () => {
		assert.equal(
			rewrite_js(
				[
					`import {a} from './a.ts';`,
					`// import {b} from './b.ts';`,
					"const t = `import {c} from './c.ts';`;",
					`export {d} from './d.ts';`
				].join('\n')
			),
			[
				`import {a} from './a.js';`,
				`// import {b} from './b.ts';`,
				"const t = `import {c} from './c.ts';`;",
				`export {d} from './d.js';`
			].join('\n')
		);
	});

	test('rewrites a dynamic import of a template literal without substitutions', () => {
		assert.equal(
			rewrite_js('const b = await import(`./b.ts`);'),
			'const b = await import(`./b.js`);'
		);
	});

	test('leaves `require` alone, which is not an import request', () => {
		const input = `const r = require('./r.ts');`;
		assert.equal(rewrite_js(input), input);
	});

	test('rewrites beside non-ASCII text and a BOM, its offsets in UTF-16', () => {
		assert.equal(
			rewrite_js(`\uFEFF// é😀\nimport {a} from './a.ts';`),
			`\uFEFF// é😀\nimport {a} from './a.js';`
		);
	});

	test('falls back to the pattern and warns when the file does not parse', () => {
		const log = create_mock_logger();
		assert.equal(
			rewrite_dist_file_imports('broken.js', `import {a} from './a.ts';\nconst = ;\n`, log),
			`import {a} from './a.js';\nconst = ;\n`
		);
		assert.equal(vi.mocked(log.warn).mock.calls.length, 1);
		assert.include(String(vi.mocked(log.warn).mock.calls[0]![0]), 'broken.js');
	});

	test('does not warn when the file parses', () => {
		const log = create_mock_logger();
		rewrite_dist_file_imports('a.js', `import {a} from './a.ts';`, log);
		assert.equal(vi.mocked(log.warn).mock.calls.length, 0);
	});
});

describe('rewrite_dist_file_imports in `.d.ts`', () => {
	test('rewrites a type-position `import()`', () => {
		assert.equal(
			rewrite_dist_file_imports('a.d.ts', `export declare const x: import('./y.ts').Y;\n`),
			`export declare const x: import('./y.js').Y;\n`
		);
	});

	test('rewrites a `.svelte.d.ts` as a declaration file', () => {
		assert.equal(
			rewrite_dist_file_imports('Comp.svelte.d.ts', `export type A = import('./y.ts').Y;\n`),
			`export type A = import('./y.js').Y;\n`
		);
	});
});

describe('rewrite_dist_file_imports in `.svelte`', () => {
	test('rewrites specifiers inside the `<script>` block only', () => {
		const input = `<script lang="ts">
	import {strip_start} from '@fuzdev/fuz_util/string.ts';
	import {contextmenu_context} from './contextmenu_state.svelte.ts';
	import {icon_link} from './icons.ts';
	import type {SvgData} from './svg.ts';
	import Svg from './Svg.svelte';
</script>

<a href="./not-an-import.ts">{icon_link}</a>
`;
		const expected = `<script lang="ts">
	import {strip_start} from '@fuzdev/fuz_util/string.ts';
	import {contextmenu_context} from './contextmenu_state.svelte.js';
	import {icon_link} from './icons.js';
	import type {SvgData} from './svg.js';
	import Svg from './Svg.svelte';
</script>

<a href="./not-an-import.ts">{icon_link}</a>
`;
		assert.equal(rewrite_svelte(input), expected);
	});

	test('rewrites both module and instance `<script>` blocks', () => {
		const input = `<script module lang="ts">
	import {shared} from './shared.ts';
</script>
<script lang="ts">
	import {local} from './local.ts';
</script>
`;
		const expected = `<script module lang="ts">
	import {shared} from './shared.js';
</script>
<script lang="ts">
	import {local} from './local.js';
</script>
`;
		assert.equal(rewrite_svelte(input), expected);
	});

	test('preserves `$`-sequences ($$props, $:) while rewriting specifiers', () => {
		const input = `<script lang="ts">
	import {x} from './x.ts';
	const props = $$props;
	$: doubled = x;
</script>
`;
		const expected = `<script lang="ts">
	import {x} from './x.js';
	const props = $$props;
	$: doubled = x;
</script>
`;
		assert.equal(rewrite_svelte(input), expected);
	});

	test('leaves an import statement inside a `<script>` template literal as written', () => {
		const input = `<script lang="ts">
	import {x} from './x.ts';
	const t = \`import {y} from './y.ts';\`;
</script>
`;
		const expected = `<script lang="ts">
	import {x} from './x.js';
	const t = \`import {y} from './y.ts';\`;
</script>
`;
		assert.equal(rewrite_svelte(input), expected);
	});

	test('leaves an import statement inside a `<script>` comment as written', () => {
		const input = `<script lang="ts">
	// import {y} from './y.ts';
	import {x} from './x.ts';
</script>
`;
		const expected = `<script lang="ts">
	// import {y} from './y.ts';
	import {x} from './x.js';
</script>
`;
		assert.equal(rewrite_svelte(input), expected);
	});

	test('rewrites a file led by a BOM, keeping the BOM', () => {
		assert.equal(
			rewrite_svelte(
				`\uFEFF<script lang="ts">\n\timport {x} from './x.ts';\n\timport {y} from './é.ts';\n</script>\n`
			),
			`\uFEFF<script lang="ts">\n\timport {x} from './x.js';\n\timport {y} from './é.js';\n</script>\n`
		);
	});

	test('falls back to `<script>` blocks by pattern and warns when the file does not parse', () => {
		const log = create_mock_logger();
		assert.equal(
			rewrite_dist_file_imports(
				'Broken.svelte',
				`<script lang="ts">\n\timport {x} from './x.ts';\n</script>\n<div>import './not-an-import.ts'</span>\n`,
				log
			),
			`<script lang="ts">\n\timport {x} from './x.js';\n</script>\n<div>import './not-an-import.ts'</span>\n`
		);
		assert.equal(vi.mocked(log.warn).mock.calls.length, 1);
		assert.include(String(vi.mocked(log.warn).mock.calls[0]![0]), 'Broken.svelte');
	});

	test('leaves a `.svelte` with no rewritable specifiers untouched', () => {
		const input = `<script lang="ts">
	import Svg from './Svg.svelte';
	import {x} from '@fuzdev/fuz_util/x.ts';
</script>
<div>hi</div>
`;
		assert.equal(rewrite_svelte(input), input);
	});
});

describe('rewrite_dist_imports', () => {
	let dir: string;

	beforeEach(async () => {
		dir = await mkdtemp(join(tmpdir(), 'gro-dist-rewrite-'));
	});

	afterEach(async () => {
		await rm(dir, { recursive: true, force: true });
	});

	test('rewrites `.js`, `.d.ts`, `.svelte.d.ts`, and `.svelte`, leaving maps alone', async () => {
		await mkdir(join(dir, 'nested'), { recursive: true });
		await writeFile(join(dir, 'a.d.ts'), `import type {B} from './b.ts';\n`);
		await writeFile(join(dir, 'Comp.svelte.d.ts'), `import {type P} from './helpers.ts';\n`);
		await writeFile(
			join(dir, 'Comp.svelte'),
			`<script lang="ts">\n\timport {s} from './state.svelte.ts';\n</script>\n`
		);
		// runtime `.js`: a relative `.ts` specifier is rewritten, an already-`.js` one preserved
		await writeFile(join(dir, 'a.js'), `import {b} from './b.ts';\nimport {c} from './c.js';\n`);
		// compiled `.svelte.ts` → `.svelte.js`: its `.svelte.ts` specifier is rewritten too
		await writeFile(join(dir, 'state.svelte.js'), `import {x} from './x.svelte.ts';\n`);
		// source maps are skipped
		await writeFile(join(dir, 'a.js.map'), `{"sources":["../src/lib/a.ts"]}\n`);
		await writeFile(join(dir, 'a.d.ts.map'), `{"sources":["../src/lib/a.ts"]}\n`);
		await writeFile(join(dir, 'nested', 'c.d.ts'), `export * from '../a.ts';\n`);

		const result = await rewrite_dist_imports(fs_paths, dir);

		assert.equal(await readFile(join(dir, 'a.d.ts'), 'utf8'), `import type {B} from './b.js';\n`);
		assert.equal(
			await readFile(join(dir, 'Comp.svelte.d.ts'), 'utf8'),
			`import {type P} from './helpers.js';\n`
		);
		assert.equal(
			await readFile(join(dir, 'Comp.svelte'), 'utf8'),
			`<script lang="ts">\n\timport {s} from './state.svelte.js';\n</script>\n`
		);
		assert.equal(
			await readFile(join(dir, 'a.js'), 'utf8'),
			`import {b} from './b.js';\nimport {c} from './c.js';\n`
		);
		assert.equal(
			await readFile(join(dir, 'state.svelte.js'), 'utf8'),
			`import {x} from './x.svelte.js';\n`
		);
		assert.equal(
			await readFile(join(dir, 'a.js.map'), 'utf8'),
			`{"sources":["../src/lib/a.ts"]}\n`
		);
		assert.equal(
			await readFile(join(dir, 'a.d.ts.map'), 'utf8'),
			`{"sources":["../src/lib/a.ts"]}\n`
		);
		assert.equal(
			await readFile(join(dir, 'nested', 'c.d.ts'), 'utf8'),
			`export * from '../a.js';\n`
		);

		// 6 files scanned (a.d.ts, Comp.svelte.d.ts, Comp.svelte, a.js, state.svelte.js,
		// nested/c.d.ts); the two `*.map` files are excluded. All 6 rewritten.
		assert.equal(result.scanned, 6);
		assert.equal(result.rewritten, 6);
	});

	test('keeps a BOM read from disk and warns of an unparsable file by name', async () => {
		await writeFile(
			join(dir, 'Bom.svelte'),
			`\uFEFF<script>\n\timport {x} from './x.ts';\n</script>\n`
		);
		await writeFile(join(dir, 'broken.js'), `import {a} from './a.ts';\nconst = ;\n`);
		const log = create_mock_logger();

		const result = await rewrite_dist_imports(fs_paths, dir, log);

		assert.equal(
			await readFile(join(dir, 'Bom.svelte'), 'utf8'),
			`\uFEFF<script>\n\timport {x} from './x.js';\n</script>\n`
		);
		assert.equal(
			await readFile(join(dir, 'broken.js'), 'utf8'),
			`import {a} from './a.js';\nconst = ;\n`
		);
		assert.equal(result.rewritten, 2);
		assert.equal(vi.mocked(log.warn).mock.calls.length, 1);
		assert.include(String(vi.mocked(log.warn).mock.calls[0]![0]), 'broken.js');
	});

	test('returns zero rewrites for a missing directory', async () => {
		const result = await rewrite_dist_imports(fs_paths, join(dir, 'does-not-exist'));
		assert.equal(result.scanned, 0);
		assert.equal(result.rewritten, 0);
	});
});
