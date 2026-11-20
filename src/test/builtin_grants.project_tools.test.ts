import { test, assert } from 'vitest';

import { GRO_BUILTIN_TASK_GRANTS } from '#lib/gro_builtin_tasks.ts';

test('every program a project declares is `local`, and only those, so none runs from `PATH`', () => {
	const roles: Array<[string, string, string, boolean]> = [];
	for (const [task_name, grants] of Object.entries(GRO_BUILTIN_TASK_GRANTS)) {
		for (const [role, grant] of Object.entries(grants?.programs ?? {})) {
			roles.push([task_name, role, grant.name, grant.local === true]);
		}
	}
	assert.deepEqual(
		roles.sort((a, b) => (a[0] + a[1] < b[0] + b[1] ? -1 : 1)),
		[
			['check', 'git_cli', 'git', false],
			['clean', 'git_cli', 'git', false],
			['lint', 'eslint_cli', 'eslint', true],
			['sync', 'npm_cli', 'npm', false],
			['sync', 'sveltekit_cli', 'svelte-kit', true],
			['test', 'vitest_cli', 'vitest', true],
			['typecheck', 'svelte_check_cli', 'svelte-check', true],
			['typecheck', 'sveltekit_cli', 'svelte-kit', true],
			['typecheck', 'typescript_cli', 'tsc', true]
		]
	);
});
