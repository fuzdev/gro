import {test, expect} from 'vitest';

import {format_file} from '$lib/format_file.ts';

test('format ts', async () => {
	const ts_unformatted = 'hey (1)';
	const ts_formatted = 'hey(1);\n';
	expect(await format_file(ts_unformatted, {filepath: 'foo.ts'})).toBe(ts_formatted);
	expect(await format_file(ts_unformatted, {parser: 'typescript'})).toBe(ts_formatted);
});

test('format svelte', async () => {
	const svelte_unformatted = '<style>a{color: red}</style>';
	const svelte_formatted = '<style>\n\ta {\n\t\tcolor: red;\n\t}\n</style>\n';
	expect(await format_file(svelte_unformatted, {filepath: 'foo.svelte'})).toBe(svelte_formatted);
	expect(await format_file(svelte_unformatted, {parser: 'svelte'})).toBe(svelte_formatted);
});
