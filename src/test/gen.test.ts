import { test, expect } from 'vitest';
import { resolve } from 'node:path';

import { to_gen_result, find_genfiles, validate_gen_module } from '$lib/gen.ts';
import { paths } from '$lib/paths.ts';
import { create_empty_gro_config } from '$lib/gro_config.ts';

const origin_id = resolve('src/foo.gen.ts');

test('to_gen_result plain string', () => {
	expect(to_gen_result(origin_id, '/**/')).toEqual({
		origin_id,
		files: [{ id: resolve('src/foo.ts'), content: '/**/', origin_id, format: true }]
	});
});

test('to_gen_result object with a content string', () => {
	expect(to_gen_result(origin_id, { content: '/**/' })).toEqual({
		origin_id,
		files: [{ id: resolve('src/foo.ts'), content: '/**/', origin_id, format: true }]
	});
});

test('to_gen_result fail with an unresolved id', () => {
	expect(() => to_gen_result('src/foo.ts', { content: '/**/' })).toThrow();
});

test('to_gen_result fail with a build id', () => {
	expect(() => to_gen_result(resolve('.gro/foo.js'), { content: '/**/' })).toThrow();
});

test('to_gen_result fail with an empty id', () => {
	expect(() => to_gen_result('', { content: '/**/' })).toThrow();
});

test('to_gen_result custom file name', () => {
	expect(
		to_gen_result(origin_id, {
			filename: 'fooz.ts',
			content: '/**/'
		})
	).toEqual({
		origin_id,
		files: [{ id: resolve('src/fooz.ts'), content: '/**/', origin_id, format: true }]
	});
});

test('to_gen_result custom file name that matches the default file name', () => {
	expect(
		to_gen_result(origin_id, {
			filename: 'foo.ts',
			content: '/**/'
		})
	).toEqual({
		origin_id,
		files: [{ id: resolve('src/foo.ts'), content: '/**/', origin_id, format: true }]
	});
});

test('to_gen_result fail when custom file name explicitly matches the origin', () => {
	expect(() => {
		to_gen_result(origin_id, {
			filename: 'foo.gen.ts',
			content: '/**/'
		});
	}).toThrow();
});

test('to_gen_result fail when file name implicitly matches the origin', () => {
	expect(() => {
		to_gen_result(resolve('src/foo.ts'), { content: '/**/' });
	}).toThrow();
});

test('to_gen_result fail with an empty file name', () => {
	expect(() => to_gen_result(origin_id, { filename: '', content: '/**/' })).toThrow();
});

test('to_gen_result additional file name parts', () => {
	expect(to_gen_result(resolve('src/foo.bar.gen.ts'), { content: '/**/' })).toEqual({
		origin_id: resolve('src/foo.bar.gen.ts'),
		files: [
			{
				id: resolve('src/foo.bar.ts'),
				content: '/**/',
				origin_id: resolve('src/foo.bar.gen.ts'),
				format: true
			}
		]
	});
});

test('to_gen_result js', () => {
	expect(
		to_gen_result(origin_id, {
			filename: 'foo.js',
			content: '/**/'
		})
	).toEqual({
		origin_id,
		files: [{ id: resolve('src/foo.js'), content: '/**/', origin_id, format: true }]
	});
});

test('to_gen_result implicit custom file extension', () => {
	expect(to_gen_result(resolve('src/foo.gen.json.ts'), '[/**/]')).toEqual({
		origin_id: resolve('src/foo.gen.json.ts'),
		files: [
			{
				id: resolve('src/foo.json'),
				content: '[/**/]',
				origin_id: resolve('src/foo.gen.json.ts'),
				format: true
			}
		]
	});
});

test('to_gen_result implicit empty file extension', () => {
	expect(to_gen_result(resolve('src/foo.gen..ts'), '[/**/]')).toEqual({
		origin_id: resolve('src/foo.gen..ts'),
		files: [
			{
				id: resolve('src/foo'),
				content: '[/**/]',
				origin_id: resolve('src/foo.gen..ts'),
				format: true
			}
		]
	});
});

test('to_gen_result implicit custom file extension with additional file name parts', () => {
	expect(to_gen_result(resolve('src/foo.bar.gen.json.ts'), { content: '[/**/]' })).toEqual({
		origin_id: resolve('src/foo.bar.gen.json.ts'),
		files: [
			{
				id: resolve('src/foo.bar.json'),
				content: '[/**/]',
				origin_id: resolve('src/foo.bar.gen.json.ts'),
				format: true
			}
		]
	});
});

test('to_gen_result implicit custom file extension with many dots in between', () => {
	expect(to_gen_result(resolve('src/foo...gen.ts'), '[/**/]')).toEqual({
		origin_id: resolve('src/foo...gen.ts'),
		files: [
			{
				id: resolve('src/foo...ts'),
				content: '[/**/]',
				origin_id: resolve('src/foo...gen.ts'),
				format: true
			}
		]
	});
});

test('to_gen_result fail with two parts following the .gen. pattern in the file name', () => {
	// This just ensures consistent file names - maybe loosen the restriction?
	// You can still implicitly name files like this,
	// but you have to move ".bar" before ".gen".
	expect(() => to_gen_result(resolve('src/foo.gen.bar.json.ts'), '/**/')).toThrow();
});

test('to_gen_result fail implicit file extension ending with a dot', () => {
	// This just ensures consistent file names - maybe loosen the restriction?
	// This one is more restrictive than the above,
	// because to have a file ending with a dot
	// you have to use an explicit file name.
	expect(() => to_gen_result(resolve('src/foo.gen...ts'), '[/**/]')).toThrow();
});

test('to_gen_result fail without a .gen. pattern in the file name', () => {
	expect(() => {
		to_gen_result(resolve('src/foo.ts'), '/**/');
	}).toThrow();
});

test('to_gen_result fail without a .gen. pattern in a file name that has multiple other patterns', () => {
	expect(() => {
		to_gen_result(resolve('src/foo.bar.baz.ts'), '/**/');
	}).toThrow();
});

test('to_gen_result fail with two .gen. patterns in the file name', () => {
	expect(() => to_gen_result(resolve('src/lib/gen.gen.ts'), '/**/')).toThrow();
	expect(() => to_gen_result(resolve('src/foo.gen.gen.ts'), '/**/')).toThrow();
	expect(() => to_gen_result(resolve('src/foo.gen.bar.gen.ts'), '/**/')).toThrow();
	expect(() => to_gen_result(resolve('src/foo.gen.bar.gen.baz.ts'), '/**/')).toThrow();
});

test('to_gen_result explicit custom file extension', () => {
	expect(
		to_gen_result(origin_id, {
			filename: 'foo.json',
			content: '[/**/]'
		})
	).toEqual({
		origin_id,
		files: [{ id: resolve('src/foo.json'), content: '[/**/]', origin_id, format: true }]
	});
});

test('to_gen_result explicit custom empty file extension', () => {
	expect(
		to_gen_result(origin_id, {
			filename: 'foo',
			content: '[/**/]'
		})
	).toEqual({
		origin_id,
		files: [{ id: resolve('src/foo'), content: '[/**/]', origin_id, format: true }]
	});
});

test('to_gen_result explicit custom file extension ending with a dot', () => {
	expect(
		to_gen_result(origin_id, {
			filename: 'foo.',
			content: '[/**/]'
		})
	).toEqual({
		origin_id,
		files: [{ id: resolve('src/foo.'), content: '[/**/]', origin_id, format: true }]
	});
});

test('to_gen_result simple array of raw files', () => {
	expect(
		to_gen_result(origin_id, [{ content: '/*1*/' }, { filename: 'foo2.ts', content: '/*2*/' }])
	).toEqual({
		origin_id,
		files: [
			{ id: resolve('src/foo.ts'), content: '/*1*/', origin_id, format: true },
			{ id: resolve('src/foo2.ts'), content: '/*2*/', origin_id, format: true }
		]
	});
});

test('to_gen_result complex array of raw files', () => {
	expect(
		to_gen_result(origin_id, [
			{ content: '/*1*/' },
			{ filename: 'foo2.ts', content: '/*2*/' },
			{ filename: 'foo3.ts', content: '/*3*/' },
			{ filename: 'foo4.ts', content: '/*4*/' },
			{ filename: 'foo5.json', content: '[/*5*/]' }
		])
	).toEqual({
		origin_id,
		files: [
			{ id: resolve('src/foo.ts'), content: '/*1*/', origin_id, format: true },
			{ id: resolve('src/foo2.ts'), content: '/*2*/', origin_id, format: true },
			{ id: resolve('src/foo3.ts'), content: '/*3*/', origin_id, format: true },
			{ id: resolve('src/foo4.ts'), content: '/*4*/', origin_id, format: true },
			{ id: resolve('src/foo5.json'), content: '[/*5*/]', origin_id, format: true }
		]
	});
});

test('to_gen_result fail with duplicate names because of omissions', () => {
	expect(() => {
		to_gen_result(origin_id, [{ content: '/*1*/' }, { content: '/*2*/' }]);
	}).toThrow();
});

test('to_gen_result fail with duplicate explicit names', () => {
	expect(() => {
		to_gen_result(origin_id, [
			{ filename: 'foo.ts', content: '/*1*/' },
			{ filename: 'foo.ts', content: '/*2*/' }
		]);
	}).toThrow();
});

test('to_gen_result fail with duplicate explicit and implicit names', () => {
	expect(() => {
		to_gen_result(origin_id, [{ content: '/*1*/' }, { filename: 'foo.ts', content: '/*2*/' }]);
	}).toThrow();
});

test('validate_gen_module basic behavior', () => {
	expect(validate_gen_module({ gen: Function.prototype })).toBeTruthy();
	expect(validate_gen_module({ gen: {} })).toBeFalsy();
	expect(validate_gen_module({ task: { run: {} } })).toBeFalsy();
});

test('find_genfiles_result finds gen modules in a directory', async () => {
	const find_genfiles_result = await find_genfiles(
		['../docs'],
		[paths.lib],
		create_empty_gro_config()
	);
	if (!find_genfiles_result.ok) {
		throw new Error('Expected find_genfiles to succeed');
	}
	expect(find_genfiles_result.value.resolved_input_paths.length).toBeTruthy();
});
