import { test, assert } from 'vitest';
import { get_schema_entries } from '@fuzdev/fuz_schema/introspect.ts';

import { GRO_BUILTIN_TASK_GRANTS } from '#lib/gro_builtin_tasks.ts';
import { create_empty_gro_config } from '#lib/gro_config.ts';

// the roles whose tool is the product, named otherwise than its program;
// every other role is its program's name, dashes dropped or made underscores, with `_cli`
const ROLE_TOOLS: Record<string, string> = { typescript_cli: 'tsc' };

test("each program role is its tool's name with `_cli`, dashes dropped or made underscores", () => {
	for (const [task_name, grants] of Object.entries(GRO_BUILTIN_TASK_GRANTS)) {
		if (!grants?.programs) continue;
		for (const [role, grant] of Object.entries(grants.programs)) {
			const label = `${task_name}'s \`${role}\``;
			assert.ok(role.endsWith('_cli'), label);
			const tool = role.slice(0, -'_cli'.length);
			assert.ok(
				ROLE_TOOLS[role] === grant.name ||
					tool === grant.name.replaceAll('-', '') ||
					tool === grant.name.replaceAll('-', '_'),
				label
			);
		}
	}
});

test('no arg and no config field chooses a program', async () => {
	for (const task_name of Object.keys(GRO_BUILTIN_TASK_GRANTS)) {
		const { task } = await import(`../lib/${task_name}.task.ts`);
		if (!task.Args) continue;
		// every key the schema declares, optional ones without a default included
		const entries = get_schema_entries(task.Args);
		assert.ok(entries, task_name);
		for (const key of Object.keys(entries)) {
			assert.notOk(key.endsWith('_cli'), `${task_name}'s \`${key}\``);
		}
	}
	for (const key of Object.keys(create_empty_gro_config())) {
		assert.notOk(key.endsWith('_cli'), key);
	}
});
