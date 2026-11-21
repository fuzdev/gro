import { test, assert, vi } from 'vitest';

import { task as dev_task, type Args } from '#lib/dev.task.ts';

import { Plugins } from '#lib/plugin.ts';

import { create_mock_task_context } from './test_helpers.ts';

vi.mock('#lib/internal/node_gro_module_path.ts', () => ({
	resolve_gro_module_path: vi.fn((path: string) => `/resolved_gro/${path}`)
}));

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

test('hands its plugins the loader of the Gro it runs, as `host.loader_path`', async () => {
	const create = vi.spyOn(Plugins, 'create');
	const ctx = create_mock_task_context<Args>({
		watch: false,
		'no-watch': true,
		sync: false,
		'no-sync': true,
		install: false
	});
	await dev_task.run(ctx);
	assert.strictEqual(create.mock.calls[0]![0].host.loader_path, '/resolved_gro/loader.js');
	create.mockRestore();
});
