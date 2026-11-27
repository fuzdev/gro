import { test, assert, vi, afterEach } from 'vitest';

import { task } from '#lib/test.task.ts';
import { project_has_sveltekit_app } from '#lib/sveltekit_helpers.ts';
import { create_mock_builtin_context } from './test_helpers.ts';
import { get_mock_spawn } from './spawn_mocks.ts';

vi.mock('#lib/spawn.ts', async () => (await import('./spawn_mocks.ts')).create_mock_spawn_module());

vi.mock('#lib/package_json.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/package_json.ts')>()),
	project_has_dependency: vi.fn(async () => true)
}));

vi.mock('#lib/sveltekit_helpers.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/sveltekit_helpers.ts')>()),
	project_has_sveltekit_app: vi.fn(async () => true)
}));

afterEach(async () => {
	vi.clearAllMocks();
	(await get_mock_spawn()).reset();
});

const ARGS = { _: ['.test.'], dir: 'src', fail_without_tests: true, sync: true };

// the programs the test task started, in order, each by its name and first arg
const started = async (args: object, sveltekit = true): Promise<Array<string>> => {
	vi.mocked(project_has_sveltekit_app).mockResolvedValue(sveltekit);
	await task.run(await create_mock_builtin_context(task, { ...ARGS, ...args }));
	return (await get_mock_spawn()).calls.map((call) => `${call.name} ${call.args[0]}`);
};

test('a SvelteKit project is synced before vitest, so vitest writes nothing of `$app`', async () => {
	assert.deepEqual(await started({}), [
		'node_modules/.bin/svelte-kit sync',
		'node_modules/.bin/vitest run'
	]);
});

test('`sync: false`, as `gro check` passes after its typecheck synced, starts vitest alone', async () => {
	assert.deepEqual(await started({ sync: false }), ['node_modules/.bin/vitest run']);
});

test('a project without SvelteKit starts vitest alone', async () => {
	assert.deepEqual(await started({}, false), ['node_modules/.bin/vitest run']);
});
