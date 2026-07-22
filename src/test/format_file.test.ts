import { test, expect } from 'vitest';

import { format_file } from '$lib/format_file.ts';

test('format ts', () => {
	const ts_unformatted = 'hey (1)';
	const ts_formatted = 'hey(1);\n';
	expect(format_file(ts_unformatted, { filepath: 'foo.ts' })).toBe(ts_formatted);
	expect(format_file(ts_unformatted, { lang: 'typescript' })).toBe(ts_formatted);
});

test('format js', () => {
	expect(format_file('const x=1', { filepath: 'foo.js' })).toBe('const x = 1;\n');
});

test('format svelte', () => {
	const svelte_unformatted = '<style>a{color: red}</style>';
	const svelte_formatted = '<style>\n\ta {\n\t\tcolor: red;\n\t}\n</style>\n';
	expect(format_file(svelte_unformatted, { filepath: 'foo.svelte' })).toBe(svelte_formatted);
	expect(format_file(svelte_unformatted, { lang: 'svelte' })).toBe(svelte_formatted);
});

test('format css', () => {
	expect(format_file('a{color:red}', { filepath: 'foo.css' })).toBe('a {\n\tcolor: red;\n}\n');
});

test('json is not auto-formatted by extension', () => {
	// `gro format` and gen leave json untouched — `infer_lang` returns `null` for `.json`.
	const json = '{"b":1,"a":2}';
	expect(format_file(json, { filepath: 'foo.json' })).toBe(json);
	expect(format_file(json, { filepath: 'tsconfig.json' })).toBe(json);
});

test('explicit json lang still formats with tabs', () => {
	expect(format_file('{"b":1,"a":2}', { lang: 'json' })).toBe('{\n\t"b": 1,\n\t"a": 2\n}\n');
	// non-strict json (jsonc) passes through unchanged rather than throwing
	const jsonc = '{\n\t// a comment\n\t"a": 1\n}';
	expect(format_file(jsonc, { lang: 'json' })).toBe(jsonc);
});

test('unsupported extension passes through unchanged', () => {
	const md = '# hi\n\n\nextra';
	expect(format_file(md, { filepath: 'foo.md' })).toBe(md);
	expect(format_file(md)).toBe(md);
});

test('invalid source throws', () => {
	expect(() => format_file('const = =', { filepath: 'foo.ts' })).toThrow();
});
