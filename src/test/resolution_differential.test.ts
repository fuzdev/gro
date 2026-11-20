import { test, assert, vi } from 'vitest';
import { spawnSync } from 'node:child_process';
import { join } from 'node:path';
import { paths } from './test_helpers.ts';

/*

The Node half of the resolver's differential cases (`crates/gro_runtime/tests/fixtures/resolution`):
each case of `cases.json` resolved under this Node, checked against the committed
`expected.json`, which the runtime's resolver test holds `fgro` to. A Node that resolves a case
otherwise fails here until the file is regenerated, and the regenerated file then fails the
resolver where it hasn't followed or declared the departure.

*/

vi.setConfig({ testTimeout: 60_000 });

const ORACLE = join(paths.root, 'crates/gro_runtime/tests/fixtures/resolution/oracle.js');

test('Node resolves what the resolver differential cases expect', () => {
	const { status, stdout, stderr } = spawnSync(process.execPath, [ORACLE, '--check'], {
		encoding: 'utf8'
	});
	assert.strictEqual(status, 0, stderr || stdout);
});
