import { test, assert } from 'vitest';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

import { GRO_BUILTIN_TASK_GRANTS, GRO_CLI_GRANTS } from '#lib/gro_builtin_tasks.ts';
import { get_schema_fields, unwrap_schema } from '@fuzdev/fuz_schema/introspect.ts';
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

test('the JSON has only its description, the two tables, and the args', () => {
	assert.deepEqual(Object.keys(read_json()), ['description', 'cli', 'builtins', 'args']);
});

test("the args are each builtin's `Args`, a boolean's default or `null`", async () => {
	const { builtins, args } = read_json() as {
		builtins: Record<string, unknown>;
		args: Record<string, Record<string, boolean | null> | null>;
	};
	assert.deepEqual(Object.keys(args), Object.keys(builtins));
	for (const name of Object.keys(builtins)) {
		const { task } = await import(`#lib/${name}.task.ts`);
		const record = args[name];
		if (!task.Args) {
			assert.isNull(record, name);
			continue;
		}
		assert.isNotNull(record, name);
		const fields = get_schema_fields(task.Args);
		assert.deepEqual(
			Object.keys(record!),
			fields.map((f) => f.name),
			name
		);
		for (const field of fields) {
			const expected = unwrap_schema(field.schema).kind === 'boolean' ? field.default : null;
			assert.strictEqual(record![field.name], expected, `${name}: ${field.name}`);
		}
	}
	// `check`'s steps, each on by default with its `no-` dual, and `build` off
	const check = args.check!;
	assert.include(check, { test: true, 'no-test': false, gen: true, build: false });
});
