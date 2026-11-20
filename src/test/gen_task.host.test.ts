import { test, assert, vi } from 'vitest';

import * as s from '@fuzdev/fuz_schema/schema.ts';

import { Args, task } from '#lib/gen.task.ts';
import { load_genfiles } from '#lib/gen.ts';
import { run_gen } from '#lib/run_gen.ts';
import { format_file } from '#lib/format_file.ts';
import { cwd } from '#lib/cwd.ts';
import type { BuiltinTaskContext } from '#lib/builtin_host.ts';
import { fs } from '#lib/fs.ts';
import { create_fs_paths } from '#lib/fs_paths.ts';

import { create_mock_task_context, create_mock_svelte_config } from './test_helpers.ts';
import { to_fs_paths_options } from './fs_test_helpers.ts';

vi.mock('#lib/gen.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/gen.ts')>()),
	find_genfiles: vi.fn(async () => ({ ok: true, value: { resolved_input_files: [] } })),
	load_genfiles: vi.fn(async () => ({ ok: true, value: { modules: [] } }))
}));

vi.mock('#lib/run_gen.ts', () => ({
	run_gen: vi.fn(async () => ({
		results: [],
		successes: [],
		failures: [],
		input_count: 0,
		output_count: 0,
		elapsed: 0
	}))
}));

test("`gen`'s `run` loads the genfiles through its `host`, giving them no `invoke_task`", async () => {
	const svelte_config = Promise.resolve(create_mock_svelte_config());
	const host = {
		fs_paths: create_fs_paths(fs, to_fs_paths_options()),
		import_module: vi.fn()
	};
	const ctx = {
		...create_mock_task_context<Args>({ _: ['src'], root_dirs: [process.cwd()], check: true }),
		svelte_config,
		host
	} as unknown as BuiltinTaskContext<Args, typeof task.grants>;
	await task.run(ctx);
	assert.strictEqual(vi.mocked(load_genfiles).mock.calls[0]![2], host.import_module);
	const base = vi.mocked(run_gen).mock.calls[0]![1];
	// genfiles run as project code and invoke no task
	assert.notProperty(base, 'invoke_task');
	assert.strictEqual(base.load_svelte_config(), svelte_config);
	assert.strictEqual(base.config, ctx.config);
	// what the genfiles return is formatted
	assert.strictEqual(vi.mocked(run_gen).mock.calls[0]![3], format_file);
});

test("`gen`'s `run` reads its context's `svelte_config` only when a genfile does", async () => {
	const svelte_config = Promise.resolve(create_mock_svelte_config());
	let reads = 0;
	const ctx = {
		...create_mock_task_context<Args>({ _: ['src'], root_dirs: [process.cwd()], check: true }),
		get svelte_config() {
			reads++;
			return svelte_config;
		},
		host: {
			fs_paths: create_fs_paths(fs, to_fs_paths_options()),
			import_module: vi.fn()
		}
	} as unknown as BuiltinTaskContext<Args, typeof task.grants>;
	vi.mocked(run_gen).mockClear();
	await task.run(ctx);
	assert.strictEqual(reads, 0);
	assert.strictEqual(vi.mocked(run_gen).mock.calls[0]![1].load_svelte_config(), svelte_config);
	assert.strictEqual(reads, 1);
});

test("`gen`'s `root_dirs` default to the working directory", () => {
	assert.deepEqual(s.parse_or_throw(Args, {}).root_dirs, [cwd()]);
});
