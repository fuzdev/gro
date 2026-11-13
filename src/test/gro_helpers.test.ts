import { describe, test, assert, onTestFinished } from 'vitest';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

import { spawn_with_loader_args } from '#lib/gro_helpers.ts';

const DATA_URL_PREFIX = 'data:text/javascript,';

// characters a URL or a JS string would read as syntax
const HOSTILE_PATHS = [
	'/opt/a#b/loader.js',
	'/opt/a%20b/loader.js',
	'/opt/a b/loader.js',
	'/opt/a"b/loader.js',
	"/opt/a'b/loader.js",
	'/opt/a\\b/loader.js',
	'/opt/a?b/loader.js',
	'/opt/a\nb/loader.js'
];

describe('spawn_with_loader_args', () => {
	test('imports a data URL whose source registers the loader by its file URL', () => {
		for (const loader_path of HOSTILE_PATHS) {
			const args = spawn_with_loader_args(loader_path, 'main.ts', ['a'], undefined);
			assert.strictEqual(args[0], '--import');
			const url = args[1]!;
			assert.ok(url.startsWith(DATA_URL_PREFIX), url);
			const parsed = new URL(url);
			assert.strictEqual(parsed.hash, '', `no fragment for ${loader_path}`);
			assert.strictEqual(parsed.search, '', `no query for ${loader_path}`);
			const source = decodeURIComponent(url.slice(DATA_URL_PREFIX.length));
			assert.strictEqual(
				source,
				`import {register} from "node:module"; register(${JSON.stringify(pathToFileURL(loader_path).href)});`
			);
			assert.deepEqual(args.slice(-2), ['main.ts', 'a']);
		}
	});

	test('runs node with a loader and script under a directory whose name holds URL syntax', () => {
		const dir = realpathSync(mkdtempSync(join(tmpdir(), 'gro_loader_#a%20b "c\'d ')));
		onTestFinished(() => rmSync(dir, { recursive: true, force: true }));
		const loader_path = join(dir, 'loader.mjs');
		const script_path = join(dir, 'main.mjs');
		const script_url = pathToFileURL(script_path).href;
		// the loader replaces the script's source, so its output proves the loader ran
		writeFileSync(
			loader_path,
			`export const load = async (url, context, next) =>
				url === ${JSON.stringify(script_url)}
					? {format: 'module', shortCircuit: true, source: 'console.log("loaded by the loader");'}
					: next(url, context);\n`
		);
		writeFileSync(script_path, 'console.log("loaded without the loader");\n');

		const result = spawnSync(
			process.execPath,
			spawn_with_loader_args(loader_path, script_path, [], 'production'),
			{ encoding: 'utf8', env: { PATH: '/usr/bin:/bin' }, timeout: 30_000 }
		);
		assert.strictEqual(result.status, 0, result.stderr);
		assert.strictEqual(result.stdout.trim(), 'loaded by the loader');
	});
});
