import { describe, test, expect } from 'vitest';

import { is_external_module } from '$lib/module.ts';

describe('is_external_module', () => {
	test('internal browser module patterns', () => {
		expect(is_external_module('./foo')).toBe(false);
		expect(is_external_module('./foo.js')).toBe(false);
		expect(is_external_module('../foo')).toBe(false);
		expect(is_external_module('../foo.js')).toBe(false);
		expect(is_external_module('../../../foo')).toBe(false);
		expect(is_external_module('../../../foo.js')).toBe(false);
		expect(is_external_module('/foo')).toBe(false);
		expect(is_external_module('/foo.js')).toBe(false);
		expect(is_external_module('src/foo')).toBe(false);
		expect(is_external_module('src/foo.js')).toBe(false);
		expect(is_external_module('$lib/foo')).toBe(false);
		expect(is_external_module('$lib/foo.js')).toBe(false);
		expect(is_external_module('./foo/bar/baz')).toBe(false);
		expect(is_external_module('./foo/bar/baz.js')).toBe(false);
		expect(is_external_module('../foo/bar/baz')).toBe(false);
		expect(is_external_module('../foo/bar/baz.js')).toBe(false);
		expect(is_external_module('../../../foo/bar/baz')).toBe(false);
		expect(is_external_module('../../../foo/bar/baz.js')).toBe(false);
		expect(is_external_module('/foo/bar/baz')).toBe(false);
		expect(is_external_module('/foo/bar/baz.js')).toBe(false);
		expect(is_external_module('src/foo/bar/baz')).toBe(false);
		expect(is_external_module('src/foo/bar/baz.js')).toBe(false);
		expect(is_external_module('$lib/foo/bar/baz')).toBe(false);
		expect(is_external_module('$lib/foo/bar/baz.js')).toBe(false);
	});

	test('external browser module patterns', () => {
		expect(is_external_module('foo')).toBe(true);
		expect(is_external_module('foo.js')).toBe(true);
		expect(is_external_module('foo/bar/baz')).toBe(true);
		expect(is_external_module('foo/bar/baz.js')).toBe(true);
		expect(is_external_module('@foo/bar/baz')).toBe(true);
		expect(is_external_module('@foo/bar/baz.js')).toBe(true);
	});
});
