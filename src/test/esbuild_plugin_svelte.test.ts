import { test, expect } from 'vitest';
import * as esbuild from 'esbuild';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { esbuild_plugin_svelte } from '#lib/esbuild_plugin_svelte.ts';
import { esbuild_plugin_sveltekit_local_imports } from '#lib/esbuild_plugin_sveltekit_local_imports.ts';

// Passed literally rather than read off the project's config,
// which would cost a full Vite config resolution for a value these tests already know.
const base_url = '';

// TODO improve these tests to have automatic caching

test('build for the client', async () => {
	const outfile = './src/test/fixtures/modules/some_test_server_bundle_DELETEME.js';
	const built = await esbuild.build({
		entryPoints: ['./src/test/fixtures/modules/some_test_server.ts'],
		plugins: [
			esbuild_plugin_svelte({
				dev: true,
				base_url,
				svelte_compile_options: { generate: 'client' }
			})
		],
		outfile,
		format: 'esm',
		platform: 'node',
		packages: 'external',
		bundle: true,
		target: 'esnext'
	});
	expect(built.errors.length).toBe(0);
	expect(built.warnings.length).toBe(0);

	const built_output = await readFile(outfile, 'utf8');
	await rm(outfile); // TODO could be cleaner
	expect(built_output).toBe(
		`// src/test/fixtures/modules/some_test_svelte_ts.svelte.ts
import * as $ from "svelte/internal/client";
var SomeTestSvelteTs = class {
  #a = $.state("ok");
  get a() {
    return $.get(this.#a);
  }
  set a(value) {
    $.set(this.#a, value, true);
  }
};

// src/test/fixtures/modules/some_test_svelte_js.svelte.js
import * as $2 from "svelte/internal/client";
var SomeTestSvelteJs = class {
  #a = $2.state("ok");
  get a() {
    return $2.get(this.#a);
  }
  set a(value) {
    $2.set(this.#a, value, true);
  }
};

// src/test/fixtures/modules/some_test_ts.ts
var some_test_ts = ".ts";

// src/test/fixtures/modules/some_test_js.js
var some_test_js = ".js";

// src/test/fixtures/modules/some_test_server.ts
var some_test_server = "some_test_server";
export {
  SomeTestSvelteJs,
  SomeTestSvelteTs,
  some_test_js,
  some_test_server,
  some_test_ts
};
`
	);
});

test('build for the server', async () => {
	const outfile = './src/test/fixtures/modules/some_test_client_bundle_DELETEME.js';
	const built = await esbuild.build({
		entryPoints: ['./src/test/fixtures/modules/some_test_server.ts'],
		plugins: [
			esbuild_plugin_svelte({
				dev: true,
				base_url
			})
		],
		outfile,
		format: 'esm',
		platform: 'node',
		packages: 'external',
		bundle: true,
		target: 'esnext'
	});
	expect(built.errors.length).toBe(0);
	expect(built.warnings.length).toBe(0);

	const built_output = await readFile(outfile, 'utf8');
	await rm(outfile); // TODO could be cleaner
	expect(built_output).toBe(
		`// src/test/fixtures/modules/some_test_svelte_ts.svelte.ts
import * as $ from "svelte/internal/server";
var SomeTestSvelteTs = class {
  a = "ok";
};

// src/test/fixtures/modules/some_test_svelte_js.svelte.js
import * as $2 from "svelte/internal/server";
var SomeTestSvelteJs = class {
  a = "ok";
};

// src/test/fixtures/modules/some_test_ts.ts
var some_test_ts = ".ts";

// src/test/fixtures/modules/some_test_js.js
var some_test_js = ".js";

// src/test/fixtures/modules/some_test_server.ts
var some_test_server = "some_test_server";
export {
  SomeTestSvelteJs,
  SomeTestSvelteTs,
  some_test_js,
  some_test_server,
  some_test_ts
};
`
	);
});

test('a relative runes module import builds through the local-imports plugin, like a server build', async () => {
	const dir = await mkdtemp(join(tmpdir(), 'gro_esbuild_plugin_svelte_'));
	try {
		await writeFile(
			join(dir, 'entry.ts'),
			`import {State} from './state.svelte.ts';\nexport const state = new State();\n`
		);
		await writeFile(join(dir, 'state.svelte.ts'), `export class State {\n\ta = $state('ok');\n}\n`);
		const built = await esbuild.build({
			entryPoints: [join(dir, 'entry.ts')],
			// the server plugin's order: the svelte plugin loads runes modules
			// in the namespaces the local-imports plugin resolves them to
			plugins: [
				esbuild_plugin_svelte({ dev: true, base_url, dir }),
				esbuild_plugin_sveltekit_local_imports()
			],
			write: false,
			format: 'esm',
			platform: 'node',
			packages: 'external',
			bundle: true,
			target: 'esnext',
			logLevel: 'silent'
		});
		expect(built.errors.length).toBe(0);
		const output = built.outputFiles[0]!.text;
		expect(output).toContain('from "svelte/internal/server"');
		expect(output).not.toContain('$state(');
	} finally {
		await rm(dir, { recursive: true, force: true });
	}
});
