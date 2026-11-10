import { test, assert, vi, beforeEach } from 'vitest';
import type { PackageJson } from '@fuzdev/fuz_util/package_json.ts';

import { task } from '#lib/test.task.ts';
import { is_task_error } from '#lib/task.ts';

import { create_mock_task_context } from './test_helpers.ts';
import { get_mock_spawn } from './spawn_mocks.ts';
import { set_mock_process_args } from './process_mocks.ts';

vi.mock('#lib/process.ts', async () =>
	(await import('./process_mocks.ts')).create_mock_process_module()
);

vi.mock('#lib/spawn.ts', async () => (await import('./spawn_mocks.ts')).create_mock_spawn_module());

vi.mock('#lib/package_json.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/package_json.ts')>()),
	package_json_load: vi.fn()
}));

const run_with = async (package_json: Partial<PackageJson>): Promise<unknown> => {
	const { package_json_load } = vi.mocked(await import('#lib/package_json.ts'));
	package_json_load.mockResolvedValue({ name: 'a', version: '0', ...package_json });
	try {
		await task.run(
			create_mock_task_context({ _: ['.test.'], dir: 'src', fail_without_tests: false })
		);
	} catch (error) {
		return error;
	}
	return null;
};

const NO_RUNNER = 'no test runner found, add vitest to devDependencies';

beforeEach(async () => {
	vi.clearAllMocks();
	set_mock_process_args(['test']);
	(await get_mock_spawn()).reset();
});

const assert_no_runner = async (error: unknown): Promise<void> => {
	assert(is_task_error(error));
	assert.strictEqual(error.message, NO_RUNNER);
	assert.strictEqual((await get_mock_spawn()).calls.length, 0);
};

test('a dev dep runs vitest', async () => {
	assert.strictEqual(await run_with({ devDependencies: { vitest: '5' } }), null);
	assert.strictEqual((await get_mock_spawn()).calls.length, 1);
});

test('a dep runs vitest', async () => {
	assert.strictEqual(await run_with({ dependencies: { vitest: '5' } }), null);
	assert.strictEqual((await get_mock_spawn()).calls.length, 1);
});

test('a project with no vitest dep throws', async () => {
	await assert_no_runner(await run_with({}));
});

test('a project only peered on vitest throws', async () => {
	await assert_no_runner(await run_with({ peerDependencies: { vitest: '5' } }));
});

test('reads package.json with the task log, so a malformed one is reported', async () => {
	await run_with({ devDependencies: { vitest: '5' } });
	const { package_json_load } = vi.mocked(await import('#lib/package_json.ts'));
	assert.isDefined(package_json_load.mock.calls[0]![4]);
});

test('declared but not installed gives the install hint', async () => {
	(await get_mock_spawn()).available = () => false;
	const error = await run_with({ devDependencies: { vitest: '5' } });
	assert(is_task_error(error));
	assert.strictEqual(error.message, 'vitest is a dependency but not installed; run `npm i`?');
});
