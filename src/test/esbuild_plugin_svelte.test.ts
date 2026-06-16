import {test, expect} from 'vitest';
import * as esbuild from 'esbuild';
import {readFile, rm} from 'node:fs/promises';

import {esbuild_plugin_svelte} from '$lib/esbuild_plugin_svelte.ts';
import {default_svelte_config} from '$lib/svelte_config.ts';

// TODO improve these tests to have automatic caching

test('build for the client', async () => {
	const outfile = './src/test/fixtures/modules/some_test_server_bundle_DELETEME.js';
	const built = await esbuild.build({
		entryPoints: ['./src/test/fixtures/modules/some_test_server.ts'],
		plugins: [
			esbuild_plugin_svelte({
				dev: true,
				base_url: default_svelte_config.base_url,
				svelte_compile_options: {generate: 'client'},
			}),
		],
		outfile,
		format: 'esm',
		platform: 'node',
		packages: 'external',
		bundle: true,
		target: 'esnext',
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
`,
	);
});

test('build for the server', async () => {
	const outfile = './src/test/fixtures/modules/some_test_client_bundle_DELETEME.js';
	const built = await esbuild.build({
		entryPoints: ['./src/test/fixtures/modules/some_test_server.ts'],
		plugins: [
			esbuild_plugin_svelte({
				dev: true,
				base_url: default_svelte_config.base_url,
			}),
		],
		outfile,
		format: 'esm',
		platform: 'node',
		packages: 'external',
		bundle: true,
		target: 'esnext',
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
`,
	);
});
