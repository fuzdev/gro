import { describe, test, expect } from 'vitest';

import { plugin_replace, to_plugin_context } from '$lib/plugin.ts';
import { create_mock_task_context } from './test_helpers.ts';

describe('plugin_replace', () => {
	test('plugin_replace', () => {
		const a = { name: 'a' };
		const b = { name: 'b' };
		const c = { name: 'c' };
		const plugins = [a, b, c];
		const a2 = { name: 'a' };
		const b2 = { name: 'b' };
		const c2 = { name: 'c' };
		let p = plugins;
		p = plugin_replace(p, a2);
		expect(p[0]).toBe(a2);
		expect(p[1]).toBe(b);
		expect(p[2]).toBe(c);
		p = plugin_replace(p, b2);
		expect(p[0]).toBe(a2);
		expect(p[1]).toBe(b2);
		expect(p[2]).toBe(c);
		// allows duplicate names in the array
		p = plugin_replace(p, c2, 'a');
		expect(p[0]).toBe(c2);
		expect(p[1]).toBe(b2);
		expect(p[2]).toBe(c);
		p = plugin_replace(p, a2, 'c');
		expect(p[0]).toBe(a2);
		expect(p[1]).toBe(b2);
		expect(p[2]).toBe(c);
		p = plugin_replace(p, c2);
		expect(p[0]).toBe(a2);
		expect(p[1]).toBe(b2);
		expect(p[2]).toBe(c2);
	});

	test('plugin_replace without an array', () => {
		const a = { name: 'a' };
		const a2 = { name: 'a' };
		const p = plugin_replace([a], a2);
		expect(p[0]).toBe(a2);
	});

	test('plugin_replace throws if it cannot find the given name', () => {
		const a = { name: 'a' };
		const plugins = [a];
		let err;
		try {
			plugin_replace(plugins, { name: 'b' });
		} catch (_err) {
			err = _err;
		}
		expect(err).toBeTruthy();
	});
});

describe('to_plugin_context', () => {
	test('adds `dev` and `watch` and carries the task context through', () => {
		const ctx = create_mock_task_context({ a: 1 });
		const plugin_ctx = to_plugin_context(ctx, true, false);
		expect(plugin_ctx.dev).toBe(true);
		expect(plugin_ctx.watch).toBe(false);
		expect(plugin_ctx.args).toBe(ctx.args);
		expect(plugin_ctx.config).toBe(ctx.config);
		expect(plugin_ctx.filer).toBe(ctx.filer);
		expect(plugin_ctx.log).toBe(ctx.log);
		expect(plugin_ctx.timings).toBe(ctx.timings);
		expect(plugin_ctx.invoke_task).toBe(ctx.invoke_task);
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

		const plugin_ctx = to_plugin_context(ctx, false, false);
		expect(reads).toBe(0);

		await expect(plugin_ctx.svelte_config).resolves.toBe('config');
		expect(reads).toBe(1);
	});
});
