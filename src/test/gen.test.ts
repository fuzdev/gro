import { describe, test, expect } from 'vitest';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';

import {
	to_gen_result,
	find_genfiles,
	is_gen_path,
	normalize_gen_config,
	validate_gen_module,
	type GenInputs
} from '#lib/gen.ts';
import { paths } from '#lib/paths.ts';
import { is_task_error } from '#lib/task.ts';
import { create_empty_gro_config } from '#lib/gro_config.ts';
import { create_project_fs_paths } from './fs_test_helpers.ts';

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

test('to_gen_result refuses a result whose content is not a string, naming the genfile and the result', () => {
	// like a stale `export const gen = gen_task_registry;`, whose call returns a `GenConfig`
	const stale = { generate: () => '', inputs: {} };
	for (const [raw, described] of [
		[stale, 'an object with keys `generate`, `inputs` whose `content` is missing'],
		[{ content: 1 }, 'an object with keys `content` whose `content` is a number'],
		[{ content: null }, 'an object with keys `content` whose `content` is null'],
		[undefined, 'undefined'],
		[1, 'a number'],
		[['ok', stale], 'an object with keys `generate`, `inputs` whose `content` is missing']
	] as const) {
		expect(() => to_gen_result(origin_id, raw as any)).toThrow(
			`Genfile ./src/foo.gen.ts returned ${described},`
		);
	}
});

test('validate_gen_module basic behavior', () => {
	expect(validate_gen_module({ gen: Function.prototype })).toBeTruthy();
	expect(validate_gen_module({ gen: {} })).toBeFalsy();
	expect(validate_gen_module({ task: { run: {} } })).toBeFalsy();
});

test('find_genfiles_result finds gen modules in a directory', async () => {
	const find_genfiles_result = await find_genfiles(
		await create_project_fs_paths(paths.root),
		['../docs'],
		[paths.lib],
		create_empty_gro_config()
	);
	if (!find_genfiles_result.ok) {
		throw new Error('Expected find_genfiles to succeed');
	}
	expect(find_genfiles_result.value.resolved_input_paths.length).toBeTruthy();
});

test('find_genfiles finds gen modules in a project under a dot-directory', async () => {
	const tmp = mkdtempSync(join(tmpdir(), 'gro_gen_'));
	try {
		const dir = join(tmp, '.cache/project');
		for (const file of ['src/lib/a.gen.ts', 'src/.hidden/b.gen.ts', 'src/dist/c.gen.ts']) {
			mkdirSync(dirname(join(dir, file)), { recursive: true });
			writeFileSync(join(dir, file), 'export const gen = () => "";\n');
		}
		const result = await find_genfiles(
			await create_project_fs_paths(dir),
			[join(dir, 'src')],
			[dir],
			create_empty_gro_config()
		);
		if (!result.ok) throw new Error('Expected find_genfiles to succeed');
		expect(result.value.resolved_input_files.map((f) => f.id)).toEqual([
			join(dir, 'src/lib/a.gen.ts')
		]);
	} finally {
		rmSync(tmp, { recursive: true, force: true });
	}
});

test('find_genfiles finds gen modules outside the root under a dot-directory', async () => {
	const tmp = mkdtempSync(join(tmpdir(), 'gro_gen_'));
	try {
		const root = join(tmp, 'project');
		const other = join(tmp, '.cache/other');
		mkdirSync(root, { recursive: true });
		for (const file of ['src/lib/a.gen.ts', 'src/.hidden/b.gen.ts']) {
			mkdirSync(dirname(join(other, file)), { recursive: true });
			writeFileSync(join(other, file), 'export const gen = () => "";\n');
		}
		const result = await find_genfiles(
			await create_project_fs_paths(root),
			[join(other, 'src')],
			[root],
			create_empty_gro_config()
		);
		if (!result.ok) throw new Error('Expected find_genfiles to succeed');
		expect(result.value.resolved_input_files.map((f) => f.id)).toEqual([
			join(other, 'src/lib/a.gen.ts')
		]);
	} finally {
		rmSync(tmp, { recursive: true, force: true });
	}
});

test('find_genfiles filters relative to the root, whichever directory it searches', async () => {
	const dir = mkdtempSync(join(tmpdir(), 'gro_gen_'));
	try {
		for (const file of ['src/lib/a.gen.ts', 'src/lib/fixtures/b.gen.ts', 'src/test/c.gen.ts']) {
			mkdirSync(dirname(join(dir, file)), { recursive: true });
			writeFileSync(join(dir, file), 'export const gen = () => "";\n');
		}
		const config = create_empty_gro_config();
		config.search_filters = [
			(p) => !p.startsWith('src/lib/fixtures'),
			(p) => !/^src\/(test|spec)\//.test(p) // a root-anchored regexp
		];
		for (const input of ['src', 'src/lib']) {
			const result = await find_genfiles(
				await create_project_fs_paths(dir),
				[join(dir, input)],
				[dir],
				config
			);
			if (!result.ok) throw new Error('Expected find_genfiles to succeed');
			expect(result.value.resolved_input_files.map((f) => f.id)).toEqual([
				join(dir, 'src/lib/a.gen.ts')
			]);
		}
	} finally {
		rmSync(dir, { recursive: true, force: true });
	}
});

test('is_gen_path tests the file name only', () => {
	expect(is_gen_path('/a/src/lib/foo.gen.ts')).toBe(true);
	expect(is_gen_path('/a/src/lib/foo.gen.css.ts')).toBe(true);
	expect(is_gen_path('/a/my.gen.x/proj/src/lib/foo.ts')).toBe(false);
	expect(is_gen_path('src/lib/foo.ts')).toBe(false);
});

test('find_genfiles in a project under a `.gen.` directory finds only genfiles', async () => {
	const tmp = mkdtempSync(join(tmpdir(), 'gro_gen_'));
	try {
		const dir = join(tmp, 'my.gen.x/proj');
		for (const file of ['src/lib/a.gen.ts', 'src/lib/b.ts']) {
			mkdirSync(dirname(join(dir, file)), { recursive: true });
			writeFileSync(join(dir, file), 'export const gen = () => "";\n');
		}
		const result = await find_genfiles(
			await create_project_fs_paths(dir),
			[join(dir, 'src')],
			[dir],
			create_empty_gro_config()
		);
		if (!result.ok) throw new Error('Expected find_genfiles to succeed');
		expect(result.value.resolved_input_files.map((f) => f.id)).toEqual([
			join(dir, 'src/lib/a.gen.ts')
		]);
	} finally {
		rmSync(tmp, { recursive: true, force: true });
	}
});

describe('normalize_gen_config inputs', () => {
	const ROOT = '/project/';
	const ID = '/project/src/a.gen.ts';
	const generate = () => 'content';
	const normalize = (inputs: unknown) =>
		normalize_gen_config({ generate, inputs: inputs as GenInputs }, ID, ROOT);
	const expect_refused = (inputs: unknown, ...expected: Array<string>) => {
		let error: unknown;
		try {
			normalize(inputs);
		} catch (e) {
			error = e;
		}
		expect(is_task_error(error)).toBe(true);
		const { message } = error as Error;
		expect(message).toContain('a.gen.ts');
		for (const text of expected) expect(message).toContain(text);
	};

	test('a function genfile has no inputs to validate', () => {
		expect(normalize_gen_config(generate, ID, ROOT)).toEqual({ generate });
	});

	test('accepts each valid form, keeping a leading `/` as the root anchor', () => {
		for (const inputs of [
			undefined,
			'all',
			{},
			{ files: ['./src/a.json', '../sibling.json', '/project/b.json'] },
			{ patterns: ['*.json', '/package.json', '!/src/skip.json', 'src/data/', '/elsewhere/x'] }
		]) {
			expect(normalize(inputs).inputs).toBe(inputs);
		}
	});

	test('refuses inputs of the wrong shape', () => {
		for (const inputs of ['src/*.json', ['*.json'], null, 1]) {
			expect_refused(inputs, "expected `'all'`");
		}
	});

	test('refuses `files` and `patterns` that are not arrays of strings', () => {
		expect_refused({ files: 'a.json' }, '`files` must be an array of strings');
		expect_refused({ files: [1] }, '`files` must be an array of strings');
		expect_refused({ patterns: '*.json' }, '`patterns` must be an array of strings');
		expect_refused({ patterns: [null] }, '`patterns` must be an array of strings');
	});

	test('refuses a `./` pattern, saying what to write', () => {
		expect_refused({ patterns: ['./src/*.json'] }, '`./src/*.json`', 'write `/src/*.json`');
		// anchored, as `*.json` alone would match at any depth
		expect_refused({ patterns: ['./*.json'] }, 'write `/*.json`');
		expect_refused({ patterns: ['!./src/a.json'] }, 'write `!/src/a.json`');
	});

	test('refuses a `../` pattern, pointing to `files`', () => {
		expect_refused({ patterns: ['../sibling/*.json'] }, 'in `files`');
		expect_refused({ patterns: ['!../sibling/a.json'] }, 'in `files`');
	});

	test('refuses a pattern holding the absolute root, saying what to write', () => {
		expect_refused({ patterns: ['/project/src/*.json'] }, 'write `/src/*.json`');
	});

	test('a root without a trailing slash matches only paths inside it', () => {
		const refused = () =>
			normalize_gen_config(
				{ generate, inputs: { patterns: ['/project/src/*.json'] } },
				ID,
				'/project'
			);
		expect(refused).toThrow('write `/src/*.json`');
		const inputs = { patterns: ['/projectile/x'] };
		expect(normalize_gen_config({ generate, inputs }, ID, '/project').inputs).toBe(inputs);
	});

	test('a project at `/` keeps every leading `/` as the anchor', () => {
		const inputs = { patterns: ['/src/*.json'] };
		expect(normalize_gen_config({ generate, inputs }, '/src/a.gen.ts', '/').inputs).toBe(inputs);
	});
});
