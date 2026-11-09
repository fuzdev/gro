import { test, assert, vi } from 'vitest';

import { task as release_task, type Args } from '#lib/release.task.ts';

import { create_mock_task_context } from './test_helpers.ts';

vi.mock('#lib/package_json.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/package_json.ts')>()),
	package_json_load: vi.fn(async () => ({ name: 'a', version: '0' }))
}));

vi.mock('#lib/sveltekit_helpers.ts', () => ({
	has_sveltekit_library: vi.fn(async () => ({ ok: true })),
	has_sveltekit_app: vi.fn(() => ({ ok: false }))
}));

test('--force_build reaches publish, whose build it is', async () => {
	const ctx = create_mock_task_context<Args>(
		{ force_build: true },
		{},
		{
			dry: false,
			check: true,
			'no-check': false,
			build: true,
			'no-build': false,
			pull: true,
			'no-pull': false,
			sync: true,
			'no-sync': false,
			install: true,
			'no-install': false,
			gen: true,
			'no-gen': false,
			force_build: false
		}
	);
	await release_task.run(ctx);
	const [name, args] = vi.mocked(ctx.invoke_task).mock.calls[0]!;
	assert.equal(name, 'publish');
	assert.equal((args as Record<string, unknown>).force_build, true);
});
