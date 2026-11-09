import { test, assert, vi } from 'vitest';

import { task, type Args } from '#lib/gen.task.ts';
import type { GenResults } from '#lib/gen.ts';
import { run_gen } from '#lib/run_gen.ts';
import { TaskError } from '#lib/task.ts';

import { create_mock_task_context } from './test_helpers.ts';

vi.mock('#lib/gen.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/gen.ts')>()),
	find_genfiles: vi.fn(async () => ({ ok: true, value: { resolved_input_files: [] } })),
	load_genfiles: vi.fn(async () => ({ ok: true, value: { modules: [] } }))
}));

vi.mock('#lib/run_gen.ts', () => ({ run_gen: vi.fn() }));

const ID = '/p/src/a.gen.ts';

/**
 * Runs the task to a single genfile failing with `error`,
 * returning everything it logged about the failure.
 */
const run_failing = async (error: Error): Promise<string> => {
	const failure = { ok: false as const, id: ID, reason: 'failed', error, elapsed: 1 };
	vi.mocked(run_gen).mockResolvedValue({
		results: [failure],
		successes: [],
		failures: [failure],
		input_count: 1,
		output_count: 0,
		elapsed: 1
	} satisfies GenResults);
	const ctx = create_mock_task_context<Args>({ _: [], root_dirs: [process.cwd()], check: false });
	let thrown: unknown;
	try {
		await task.run(ctx);
	} catch (e) {
		thrown = e;
	}
	assert.ok(thrown instanceof TaskError);
	assert.include(thrown.message, 'Failed to generate 1 file');
	return [...vi.mocked(ctx.log.info).mock.calls, ...vi.mocked(ctx.log.error).mock.calls]
		.flat()
		.join('\n');
};

test('a genfile failing with a `TaskError` prints its message and no stack', async () => {
	const output = await run_failing(new TaskError('the message with the fix'));
	assert.equal(output.split('the message with the fix').length - 1, 2);
	assert.notMatch(output, /^\s+at /m);
});

test('a genfile failing with any other error keeps its stack', async () => {
	const output = await run_failing(new Error('unexpected'));
	assert.match(output, /^\s+at /m);
});
