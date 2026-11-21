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

/**
 * A plain copy of a value of the TS table, each null-prototype object an ordinary one,
 * keeping an `undefined` value, which JSON can't hold, so a field it drops fails to compare.
 */
const to_plain = (value: unknown): unknown => {
	if (value === null || typeof value !== 'object') return value;
	if (Array.isArray(value)) return value.map(to_plain);
	const plain: Record<string, unknown> = {};
	for (const key of Object.keys(value)) {
		plain[key] = to_plain((value as Record<string, unknown>)[key]);
	}
	return plain;
};

test('the JSON holds the TS table, deep, in its order', () => {
	const { cli, builtins } = read_json();
	assert.deepEqual(cli, to_plain(GRO_CLI_GRANTS));
	assert.deepEqual(builtins, to_plain(GRO_BUILTIN_TASK_GRANTS));
	// key order too, the builtins sorted by name, so the JSON reads as the TS table does
	assert.strictEqual(JSON.stringify(cli), JSON.stringify(GRO_CLI_GRANTS));
	assert.strictEqual(JSON.stringify(builtins), JSON.stringify(GRO_BUILTIN_TASK_GRANTS));
});

test('the JSON has only its description and the two tables', () => {
	assert.deepEqual(Object.keys(read_json()), ['description', 'cli', 'builtins']);
});
