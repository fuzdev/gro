import { test, assert } from 'vitest';
import { get_schema_entries } from '@fuzdev/fuz_schema/introspect.ts';
import * as s from '@fuzdev/fuz_schema/schema.ts';

import { GRO_BUILTIN_TASK_GRANTS } from '#lib/gro_builtin_tasks.ts';
import { GRO_BUILTIN_TASK_IMPORTS } from '#lib/gro_builtin_task_imports.ts';
import { task as check_task, Args as CheckArgs } from '#lib/check.task.ts';

test('each invoke names a builtin, and its `when` a boolean arg of the invoking task, off by default', async () => {
	for (const [task_name, grants] of Object.entries(GRO_BUILTIN_TASK_GRANTS)) {
		if (!grants?.invokes) continue;
		for (const [invoked, invoke] of Object.entries(grants.invokes)) {
			const label = `${task_name}'s invoke of \`${invoked}\``;
			assert.ok(Object.hasOwn(GRO_BUILTIN_TASK_GRANTS, invoked), label);
			if (invoke.when === undefined) continue;
			const { task } =
				await GRO_BUILTIN_TASK_IMPORTS[task_name as keyof typeof GRO_BUILTIN_TASK_IMPORTS]();
			assert.ok(task.Args, label);
			const entries = get_schema_entries(task.Args);
			assert.ok(entries && Object.hasOwn(entries, invoke.when), label);
			const parsed = s.parse(task.Args, {});
			assert(parsed.ok, label);
			// off by default, so a runtime holds the invoke only when the command line turns it on
			assert.strictEqual(parsed.value[invoke.when], false, label);
		}
	}
});

test("`check`'s closure holds `build` only under `--build`, which it defaults off", () => {
	assert.deepEqual(GRO_BUILTIN_TASK_GRANTS.check?.invokes?.build, { when: 'build' });
	assert.deepEqual(check_task.grants.invokes.build, { when: 'build' });
	const parsed = s.parse(CheckArgs, {});
	assert(parsed.ok);
	assert.strictEqual(parsed.value.build, false);
});
