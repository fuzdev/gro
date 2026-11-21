import { describe, test, expect } from 'vitest';

import { to_plugin_context, type PluginHost } from '#lib/plugin.ts';
import { create_mock_task_context } from './test_helpers.ts';

// the handles a task opens for its plugins, each a distinct object the context must carry as is
const HOST = {
	fs_paths: {},
	env: {},
	spawn: {},
	import_module: () => Promise.reject(Error('unused'))
} as unknown as PluginHost;

describe('to_plugin_context', () => {
	test('adds `host`, `dev`, and `watch` and carries the task context through', () => {
		const ctx = create_mock_task_context({ a: 1 });
		const plugin_ctx = to_plugin_context(ctx, HOST, true, false);
		expect(plugin_ctx.host).toBe(HOST);
		expect(plugin_ctx.dev).toBe(true);
		expect(plugin_ctx.watch).toBe(false);
		expect(plugin_ctx.task_args).toBe(ctx.task_args);
		expect(plugin_ctx.config).toBe(ctx.config);
		expect(plugin_ctx.graph).toBe(ctx.graph);
		expect(plugin_ctx.log).toBe(ctx.log);
		expect(plugin_ctx.timings).toBe(ctx.timings);
		expect(plugin_ctx.invoke_task).toBe(ctx.invoke_task);
		expect(plugin_ctx.raw_rest_args).toBe(ctx.raw_rest_args);
	});

	// `svelte_config` is a lazy getter on the real task context, and spreading would call it,
	// resolving the Svelte config for plugin sets that never read it.
	test('does not read a lazy `svelte_config`', async () => {
		let reads = 0;
		const svelte_config = Promise.resolve('config');
		const ctx = Object.defineProperty(create_mock_task_context(), 'svelte_config', {
			enumerable: true,
			get: () => {
				reads++;
				return svelte_config;
			}
		});

		const plugin_ctx = to_plugin_context(ctx, HOST, false, false);
		expect(reads).toBe(0);

		await expect(plugin_ctx.svelte_config).resolves.toBe('config');
		expect(reads).toBe(1);
	});
});
