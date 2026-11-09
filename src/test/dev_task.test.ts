import { test, assert } from 'vitest';

import { task as dev_task, type Args } from '#lib/dev.task.ts';

import { create_mock_task_context } from './test_helpers.ts';

test('`gro dev --no-watch` leaves gen to the plugin, so sync does not run it too', async () => {
	const ctx = create_mock_task_context<Args>({
		watch: false,
		'no-watch': true,
		sync: true,
		'no-sync': false,
		install: false
	});
	await dev_task.run(ctx);
	assert.deepEqual((ctx.invoke_task as any).mock.calls, [['sync', { install: false, gen: false }]]);
});
