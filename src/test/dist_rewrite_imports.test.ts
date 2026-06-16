import {describe, test, assert, beforeEach, afterEach} from 'vitest';
import {mkdtemp, mkdir, writeFile, readFile, rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';

import {
	rewrite_relative_ts_imports,
	rewrite_svelte_ts_imports,
	rewrite_dist_imports,
} from '../lib/dist_rewrite_imports.ts';

describe('rewrite_relative_ts_imports', () => {
	test('rewrites relative `.ts` specifiers to `.js`', () => {
		assert.equal(
			rewrite_relative_ts_imports(`import {a} from './a.ts';`),
			`import {a} from './a.js';`,
		);
		assert.equal(
			rewrite_relative_ts_imports(`import {a} from '../parent/a.ts';`),
			`import {a} from '../parent/a.js';`,
		);
	});

	test('rewrites the `.svelte.ts` double extension to `.svelte.js`', () => {
		assert.equal(
			rewrite_relative_ts_imports(`import {s} from './state.svelte.ts';`),
			`import {s} from './state.svelte.js';`,
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
			`import type {Logger} from './log.js';`,
		);
	});

	test('rewrites `export … from` and `export * from` specifiers', () => {
		assert.equal(
			rewrite_relative_ts_imports(`export {a} from './a.ts';`),
			`export {a} from './a.js';`,
		);
		assert.equal(rewrite_relative_ts_imports(`export * from './a.ts';`), `export * from './a.js';`);
	});

	test('rewrites side-effect imports', () => {
		assert.equal(rewrite_relative_ts_imports(`import './register.ts';`), `import './register.js';`);
	});

	test('rewrites dynamic and `.d.ts` `import(...)` type specifiers', () => {
		assert.equal(
			rewrite_relative_ts_imports(`const m = await import('./lazy.ts');`),
			`const m = await import('./lazy.js');`,
		);
		assert.equal(
			rewrite_relative_ts_imports(`export declare const x: import("./types.ts").Foo;`),
			`export declare const x: import("./types.js").Foo;`,
		);
	});

	test('handles both quote styles', () => {
		assert.equal(
			rewrite_relative_ts_imports(`import {a} from "./a.ts";`),
			`import {a} from "./a.js";`,
		);
	});

	test('does not match `.tsx`, `.mts`, or query-suffixed specifiers', () => {
		const input = `import X from './X.tsx';\nimport {m} from './m.mts';\nimport raw from './r.ts?raw';`;
		assert.equal(rewrite_relative_ts_imports(input), input);
	});

	test('rewrites every specifier across a realistic `.d.ts` body', () => {
		const input = [
			`import { type ContextmenuRootBaseProps } from './contextmenu_helpers.ts';`,
			`import type {SvgData} from './svg.ts';`,
			`import {Tome} from './tome.ts';`,
			`import {strip_start} from '@fuzdev/fuz_util/string.ts';`,
			`export * from './reexport.ts';`,
		].join('\n');
		const expected = [
			`import { type ContextmenuRootBaseProps } from './contextmenu_helpers.js';`,
			`import type {SvgData} from './svg.js';`,
			`import {Tome} from './tome.js';`,
			`import {strip_start} from '@fuzdev/fuz_util/string.ts';`,
			`export * from './reexport.js';`,
		].join('\n');
		assert.equal(rewrite_relative_ts_imports(input), expected);
	});
});

describe('rewrite_svelte_ts_imports', () => {
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
		assert.equal(rewrite_svelte_ts_imports(input), expected);
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
		assert.equal(rewrite_svelte_ts_imports(input), expected);
	});

	test('leaves a `.svelte` with no rewritable specifiers untouched', () => {
		const input = `<script lang="ts">
	import Svg from './Svg.svelte';
	import {x} from '@fuzdev/fuz_util/x.ts';
</script>
<div>hi</div>
`;
		assert.equal(rewrite_svelte_ts_imports(input), input);
	});
});

describe('rewrite_dist_imports', () => {
	let dir: string;

	beforeEach(async () => {
		dir = await mkdtemp(join(tmpdir(), 'gro-dist-rewrite-'));
	});

	afterEach(async () => {
		await rm(dir, {recursive: true, force: true});
	});

	test('rewrites `.d.ts`, `.svelte.d.ts`, and `.svelte`, leaving `.js` and maps alone', async () => {
		await mkdir(join(dir, 'nested'), {recursive: true});
		await writeFile(join(dir, 'a.d.ts'), `import type {B} from './b.ts';\n`);
		await writeFile(join(dir, 'Comp.svelte.d.ts'), `import {type P} from './helpers.ts';\n`);
		await writeFile(
			join(dir, 'Comp.svelte'),
			`<script lang="ts">\n\timport {s} from './state.svelte.ts';\n</script>\n`,
		);
		// already emitted by tsc — must stay untouched
		await writeFile(join(dir, 'a.js'), `import {b} from './b.js';\n`);
		await writeFile(join(dir, 'a.d.ts.map'), `{"sources":["../src/lib/a.ts"]}\n`);
		await writeFile(join(dir, 'nested', 'c.d.ts'), `export * from '../a.ts';\n`);

		const result = await rewrite_dist_imports(dir);

		assert.equal(await readFile(join(dir, 'a.d.ts'), 'utf8'), `import type {B} from './b.js';\n`);
		assert.equal(
			await readFile(join(dir, 'Comp.svelte.d.ts'), 'utf8'),
			`import {type P} from './helpers.js';\n`,
		);
		assert.equal(
			await readFile(join(dir, 'Comp.svelte'), 'utf8'),
			`<script lang="ts">\n\timport {s} from './state.svelte.js';\n</script>\n`,
		);
		assert.equal(await readFile(join(dir, 'a.js'), 'utf8'), `import {b} from './b.js';\n`);
		assert.equal(
			await readFile(join(dir, 'a.d.ts.map'), 'utf8'),
			`{"sources":["../src/lib/a.ts"]}\n`,
		);
		assert.equal(
			await readFile(join(dir, 'nested', 'c.d.ts'), 'utf8'),
			`export * from '../a.js';\n`,
		);

		// 4 files scanned (a.d.ts, Comp.svelte.d.ts, Comp.svelte, nested/c.d.ts); all 4 rewritten
		assert.equal(result.scanned, 4);
		assert.equal(result.rewritten, 4);
	});

	test('returns zero rewrites for a missing directory', async () => {
		const result = await rewrite_dist_imports(join(dir, 'does-not-exist'));
		assert.equal(result.scanned, 0);
		assert.equal(result.rewritten, 0);
	});
});
