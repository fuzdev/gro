import { test, assert } from 'vitest';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

import { GRO_BUILTIN_TASK_GRANTS, GRO_CLI_GRANTS } from '#lib/gro_builtin_tasks.ts';
import { paths } from './test_helpers.ts';

/*

`src/builtin_grants.json` is the grants table as the binary reads it, natively,
written by the same genfile as `GRO_CLI_GRANTS` and `GRO_BUILTIN_TASK_GRANTS`,
so `gro gen --check` holds each file to its render and this holds the two renders to each other.

*/

const read_json = (): Record<string, unknown> =>
	JSON.parse(readFileSync(join(paths.root, 'src/builtin_grants.json'), 'utf8'));

test('the JSON holds the TS table, deep, in its order', () => {
	const { cli, builtins } = read_json();
	// chai compares a null-prototype object of the TS table equal to a plain one
	assert.deepEqual(cli, GRO_CLI_GRANTS);
	assert.deepEqual(builtins, GRO_BUILTIN_TASK_GRANTS);
	// key order too, the builtins sorted by name, so the JSON reads as the TS table does
	assert.strictEqual(JSON.stringify(cli), JSON.stringify(GRO_CLI_GRANTS));
	assert.strictEqual(JSON.stringify(builtins), JSON.stringify(GRO_BUILTIN_TASK_GRANTS));
});

test('the JSON has only its description and the two tables', () => {
	assert.deepEqual(Object.keys(read_json()), ['description', 'cli', 'builtins']);
});
