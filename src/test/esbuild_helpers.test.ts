import { expect, test } from 'vitest';

import { to_define_import_meta_env } from '#lib/esbuild_helpers.ts';

test('to_define_import_meta_env defaults `ssr` to true and `mode` from `dev`', () => {
	expect(Object.values(to_define_import_meta_env({ dev: true, base_url: '' }))).toEqual([
		'true',
		'false',
		'true',
		'"development"',
		'"/"'
	]);
	expect(Object.values(to_define_import_meta_env({ dev: false, base_url: '/base' }))).toEqual([
		'false',
		'true',
		'true',
		'"production"',
		'"/base"'
	]);
});
