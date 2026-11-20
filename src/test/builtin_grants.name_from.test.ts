import { test, assert } from 'vitest';
import { args_parse } from '@fuzdev/fuz_util/args.ts';

import { GRO_BUILTIN_TASK_GRANTS } from '#lib/gro_builtin_tasks.ts';
import { create_empty_gro_config } from '#lib/gro_config.ts';

test("each program role is named by the `_cli` arg or config field choosing its program, or its tool's", async () => {
	const config = create_empty_gro_config() as unknown as Record<string, unknown>;
	for (const [task_name, grants] of Object.entries(GRO_BUILTIN_TASK_GRANTS)) {
		if (!grants?.programs) continue;
		const { task } = await import(`../lib/${task_name}.task.ts`);
		for (const [role, grant] of Object.entries(grants.programs)) {
			const label = `${task_name}'s \`${role}\``;
			assert.ok(role.endsWith('_cli'), label);
			if (grant.name_from === 'arg') {
				// the task's arg of the role's own name, defaulting to the grant's name
				assert.ok(task.Args, label);
				const parsed = args_parse({}, task.Args);
				assert.ok(parsed.ok, label);
				assert.strictEqual((parsed.value as Record<string, unknown>)[role], grant.name, label);
			} else if (grant.name_from === 'config') {
				// the Gro config's field of the role's own name, defaulting to the grant's name
				assert.strictEqual(config[role], grant.name, label);
			} else {
				// a fixed tool's role is its name with `_cli`
				assert.strictEqual(role, grant.name.replaceAll('-', '') + '_cli', label);
			}
		}
	}
});
