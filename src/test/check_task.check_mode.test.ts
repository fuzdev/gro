import { test, assert, vi } from 'vitest';

import { task, type Args } from '#lib/check.task.ts';
import type { BuiltinTaskContext } from '#lib/builtin_host.ts';
import { GRO_BUILTIN_TASK_GRANTS } from '#lib/gro_builtin_tasks.ts';

import { create_mock_logger } from './test_helpers.ts';
import { create_mock_fs_paths } from './fs_mocks.ts';

// every step off but the ones a test turns on
const NONE = {
	typecheck: false,
	test: false,
	gen: false,
	format: false,
	package_json: false,
	lint: false,
	build: false,
	lockfile: false,
	install: false,
	workspace: false
} as unknown as Args;

test('`check` marks its invokes of `gen` and `format` check-mode, each a builtin that writes', () => {
	const invokes = GRO_BUILTIN_TASK_GRANTS.check?.invokes;
	assert.ok(invokes);
	const marked = Object.entries(invokes)
		.filter(([, invoke]) => invoke.check)
		.map(([name]) => name);
	assert.deepEqual(marked.sort(), ['format', 'gen']);
	for (const name of marked) {
		const grants = GRO_BUILTIN_TASK_GRANTS[name as keyof typeof GRO_BUILTIN_TASK_GRANTS];
		assert.ok(grants?.fs?.write?.length, name);
	}
});

test('`check` runs `gen` and `format` in check mode', async () => {
	const invoke_task = vi.fn();
	await task.run({
		task_args: { ...NONE, gen: true, format: true },
		config: { map_package_json: null },
		invoke_task,
		raw_rest_args: [],
		log: create_mock_logger(),
		host: {
			fs_paths: create_mock_fs_paths(),
			env: { get: () => undefined },
			programs: { program: vi.fn(), find: vi.fn(() => null) }
		}
	} as unknown as BuiltinTaskContext<Args, typeof task.grants>);
	assert.deepEqual(invoke_task.mock.calls, [
		['gen', { check: true }],
		['format', { check: true }]
	]);
});
