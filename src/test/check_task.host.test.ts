import { test, assert, vi } from 'vitest';

import { task, type Args } from '#lib/check.task.ts';
import type { BuiltinTaskContext } from '#lib/builtin_host.ts';
import { is_task_error } from '#lib/task_error.ts';

import { create_mock_logger } from './test_helpers.ts';
import { create_mock_fs_paths } from './fs_mocks.ts';

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

const create_host = () => ({
	fs_paths: create_mock_fs_paths(),
	env: { get: vi.fn(() => undefined) },
	programs: { program: vi.fn(), find: vi.fn(() => null) }
});

const run = async (
	args: Partial<Args>,
	raw_rest_args: Array<string>,
	host: ReturnType<typeof create_host> = create_host()
) => {
	const invoke_task = vi.fn();
	await task.run({
		task_args: { ...NONE, ...args },
		config: { map_package_json: null },
		invoke_task,
		raw_rest_args,
		log: create_mock_logger(),
		host
	} as unknown as BuiltinTaskContext<Args, typeof task.grants>);
	return { host, invoke_task };
};

test("`check`'s `run` reads the forwarded args from its context", async () => {
	const { invoke_task } = await run({ test: true }, ['--', 'vitest', '--reporter=dot']);
	assert.deepEqual(
		invoke_task.mock.calls.map((c) => c[0]),
		['test']
	);
});

test("`check`'s `run` reads `CI` from its `host`'s `env`", async () => {
	const { host } = await run({}, []);
	assert.deepEqual(host.env.get.mock.calls, [['CI']]);
});

test("`check`'s `run` finds git through its `host`", async () => {
	// the host finds no git, so the workspace check it asked for fails
	const host = create_host();
	let error: unknown;
	try {
		await run({ workspace: true }, [], host);
	} catch (e) {
		error = e;
	}
	assert(is_task_error(error));
	assert.deepEqual(host.programs.find.mock.calls, [['git_cli']]);
});

test("`check`'s `invoke_task` is typed by its grants' `invokes`", () => {
	const typed = (ctx: BuiltinTaskContext<Args, typeof task.grants>): void => {
		void ctx.invoke_task('gen', { check: true });
		// @ts-expect-error `check`'s grants don't name `publish`
		void ctx.invoke_task('publish');
		// @ts-expect-error a path is no builtin
		void ctx.invoke_task('src/lib/x.task.ts');
	};
	assert.isFunction(typed);
});
