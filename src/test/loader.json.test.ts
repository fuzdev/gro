import { test, assert, onTestFinished } from 'vitest';
import { spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { spawn_with_loader_args } from '#lib/gro_helpers.ts';

import { LOADER_SOURCE_PATH } from './test_helpers.ts';

/*

Imports JSON with `type: 'json'` through the real loader in a child `node`: a project file of
any extension, a package's file, which Node's resolver gives the `json` format, and a package
module importing its own JSON, each as a static and a dynamic import.

*/

test('a JSON import loads from the project and from a package, whatever the extension', () => {
	const dir = realpathSync(mkdtempSync(join(tmpdir(), 'gro_loader_json_')));
	onTestFinished(() => rmSync(dir, { recursive: true, force: true }));
	const pkg = join(dir, 'node_modules/pkg');
	mkdirSync(pkg, { recursive: true });
	writeFileSync(join(dir, 'package.json'), '{"name": "project", "type": "module"}');
	writeFileSync(join(dir, 'a.json'), '{"a": 1}');
	writeFileSync(join(dir, 'b.data'), '{"b": 2}');
	writeFileSync(
		join(pkg, 'package.json'),
		JSON.stringify({
			name: 'pkg',
			type: 'module',
			exports: { './package.json': './package.json', './c.json': './c.json', './d.js': './d.js' }
		})
	);
	writeFileSync(join(pkg, 'c.json'), '{"c": 3}');
	writeFileSync(
		join(pkg, 'd.js'),
		"import c from './c.json' with {type: 'json'}; export default c;"
	);
	const script = join(dir, 'main.ts');
	writeFileSync(
		script,
		`import a from './a.json' with {type: 'json'};
		import b from './b.data' with {type: 'json'};
		import p from 'pkg/package.json' with {type: 'json'};
		import c from 'pkg/c.json' with {type: 'json'};
		import d from 'pkg/d.js';
		const dynamic = (await import('pkg/c.json', {with: {type: 'json'}})).default;
		console.log(JSON.stringify([a, b, p.name, c, d, dynamic]));\n`
	);

	const result = spawnSync(
		process.execPath,
		spawn_with_loader_args(LOADER_SOURCE_PATH, script, [], undefined),
		{ cwd: dir, encoding: 'utf8', env: { PATH: '/usr/bin:/bin' }, timeout: 60_000 }
	);
	assert.strictEqual(result.status, 0, result.stderr);
	assert.deepEqual(JSON.parse(result.stdout), [
		{ a: 1 },
		{ b: 2 },
		'pkg',
		{ c: 3 },
		{ c: 3 },
		{ c: 3 }
	]);
});
