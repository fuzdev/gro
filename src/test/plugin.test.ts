import { describe, test, expect } from 'vitest';

import { plugin_replace } from '$lib/plugin.ts';

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
