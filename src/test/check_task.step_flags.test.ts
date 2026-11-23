import { test, assert, vi } from 'vitest';

import { run_check, task, type Args, type CheckDeps } from '#lib/check.task.ts';

import { create_mock_logger } from './test_helpers.ts';
import { create_mock_fs_paths } from './fs_mocks.ts';

/*

Each of `check`'s steps that its command line can turn off invokes its builtin only while its flag
is on, and the grants mark that invoke `when` the same flag, so a closure read from the grants
holds what `run_check` runs: a flag wired to another step's invoke fails here, not at a run.

*/

vi.mock('#lib/package_json.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/package_json.ts')>()),
	project_has_dependency: vi.fn(() => Promise.resolve(true))
}));

vi.mock('#lib/eslint_helpers.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/eslint_helpers.ts')>()),
	find_eslint_config: vi.fn(() => Promise.resolve(null))
}));

const STEPS = ['typecheck', 'test', 'gen', 'format', 'lint'] as const;

const ON = {
	typecheck: true,
	test: true,
	gen: true,
	format: true,
	package_json: false,
	lint: true,
	build: false,
	lockfile: false,
	install: false,
	workspace: false
} as unknown as Args;

const invoked_with = async (args: Partial<Args>): Promise<Array<string>> => {
	const invoke_task = vi.fn(() => Promise.resolve());
	const deps: CheckDeps = {
		fs_paths: create_mock_fs_paths(),
		env: { get: () => undefined },
		programs: { find: () => null },
		invoke_task,
		log: create_mock_logger()
	};
	await run_check({ ...ON, ...args }, { map_package_json: null, raw_rest_args: [] }, deps);
	return invoke_task.mock.calls.map((call) => (call as Array<unknown>)[0] as string);
};

test("each step's flag off drops exactly the invoke its grants mark `when` it", async () => {
	const all = await invoked_with({});
	assert.sameMembers(all, [...STEPS]);
	for (const flag of STEPS) {
		const invoked = await invoked_with({ [flag]: false });
		const dropped = all.filter((name) => !invoked.includes(name));
		assert.deepEqual(dropped, [flag], flag);
		assert.strictEqual(
			task.grants.invokes[dropped[0] as (typeof STEPS)[number]].when,
			flag,
			`the invoke of \`${dropped[0]}\``
		);
	}
});
