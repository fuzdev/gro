import { test, assert, vi } from 'vitest';
import { spawnSync } from 'node:child_process';
import { join } from 'node:path';

import { paths } from '#lib/paths.ts';

/*

The Node half of the host modules' differential cases (`crates/gro_cli/tests/fixtures/host`):
each case of `cases.json` run under this Node on the host module in `src/lib`, checked against
the committed `expected.json`, which the runtime's own tests hold each builtin module to. A
change to what a host module's exports are fails here until the file is regenerated, and the
regenerated file then fails the builtin module that hasn't followed.

*/

vi.setConfig({ testTimeout: 60_000 });

const ORACLE = join(paths.root, 'crates/gro_cli/tests/fixtures/host/oracle.js');

test('the host modules print what the differential cases expect', () => {
	const { status, stdout, stderr } = spawnSync(process.execPath, [ORACLE, '--check'], {
		encoding: 'utf8'
	});
	assert.strictEqual(status, 0, stderr || stdout);
});
