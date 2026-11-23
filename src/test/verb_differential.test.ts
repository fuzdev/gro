import { test, assert, vi } from 'vitest';
import { spawnSync } from 'node:child_process';
import { join } from 'node:path';
import { paths } from './test_helpers.ts';

/*

The Node half of the verb's differential (`crates/gro_cli/tests/fixtures/verb_differential`):
each case of `cases.json` run under this Node by the Node gro in `src/lib`, checked against the
committed `expected.json`, which the binary's own test holds `fgro <command>` to. A change to what
the Node gro prints for a case fails here until the file is regenerated, and the regenerated file
then fails the binary where it hasn't followed or declared the difference.

*/

vi.setConfig({ testTimeout: 120_000 });

const ORACLE = join(paths.root, 'crates/gro_cli/tests/fixtures/verb_differential/oracle.js');

test('the Node gro prints what the verb differential cases expect', () => {
	const { status, stdout, stderr } = spawnSync(process.execPath, [ORACLE, '--check'], {
		encoding: 'utf8'
	});
	assert.strictEqual(status, 0, stderr || stdout);
});
