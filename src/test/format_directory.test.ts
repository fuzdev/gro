import { test, expect, beforeEach, afterEach } from 'vitest';
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { format_directory, read_format_ignore } from '#lib/format_directory.ts';
import { create_empty_gro_config } from '#lib/gro_config.ts';

import { create_mock_logger } from './test_helpers.ts';

let root: string;

beforeEach(() => {
	root = mkdtempSync(join(tmpdir(), 'gro-formatignore-'));
	mkdirSync(join(root, 'src/lib'), { recursive: true });
	mkdirSync(join(root, 'src/test/fixtures/samples'), { recursive: true });
	mkdirSync(join(root, 'src/test/fixtures/generated'), { recursive: true });
	writeFileSync(join(root, 'src/lib/keep.ts'), 'const x = 1;\n');
	writeFileSync(join(root, 'src/test/fixtures/samples/sample_a.ts'), 'const a = 1;\n');
	writeFileSync(join(root, 'src/test/fixtures/samples/sample_b.svelte'), '<div />\n');
	writeFileSync(join(root, 'src/test/fixtures/generated/out.ts'), 'const g = 1;\n');
	writeFileSync(join(root, 'notes.md'), '# notes\n');
});

afterEach(() => {
	rmSync(root, { recursive: true, force: true });
});

test('no .formatignore yields an empty set', () => {
	expect(read_format_ignore(root).size).toBe(0);
});

test('anchored and non-anchored patterns expand to absolute ignored paths', () => {
	writeFileSync(
		join(root, '.formatignore'),
		'# comment\nsrc/test/fixtures/generated/**\nsrc/test/fixtures/samples/sample_*.*\n\n*.md\n'
	);
	const ignored = read_format_ignore(root);
	expect(ignored.has(resolve(root, 'src/test/fixtures/samples/sample_a.ts'))).toBe(true);
	expect(ignored.has(resolve(root, 'src/test/fixtures/samples/sample_b.svelte'))).toBe(true);
	expect(ignored.has(resolve(root, 'src/test/fixtures/generated/out.ts'))).toBe(true);
	// `*.md` has no slash, so it matches at any depth.
	expect(ignored.has(resolve(root, 'notes.md'))).toBe(true);
	// unlisted source is not ignored.
	expect(ignored.has(resolve(root, 'src/lib/keep.ts'))).toBe(false);
});

test('a blank/comment-only .formatignore yields an empty set', () => {
	writeFileSync(join(root, '.formatignore'), '# only a comment\n\n');
	expect(read_format_ignore(root).size).toBe(0);
});

test('.prettierignore is honored and merges with .formatignore', () => {
	writeFileSync(join(root, '.formatignore'), 'src/test/fixtures/samples/sample_*.*\n');
	writeFileSync(join(root, '.prettierignore'), 'src/test/fixtures/generated/**\n');
	const ignored = read_format_ignore(root);
	// from .formatignore
	expect(ignored.has(resolve(root, 'src/test/fixtures/samples/sample_a.ts'))).toBe(true);
	// from .prettierignore
	expect(ignored.has(resolve(root, 'src/test/fixtures/generated/out.ts'))).toBe(true);
	expect(ignored.has(resolve(root, 'src/lib/keep.ts'))).toBe(false);
});

test('.prettierignore alone (no .formatignore) is respected', () => {
	writeFileSync(join(root, '.prettierignore'), 'src/test/fixtures/samples/sample_*.*\n');
	const ignored = read_format_ignore(root);
	expect(ignored.has(resolve(root, 'src/test/fixtures/samples/sample_a.ts'))).toBe(true);
	expect(ignored.has(resolve(root, 'src/lib/keep.ts'))).toBe(false);
});

test('format_directory searches a project under a dot-directory', async () => {
	const dir = join(root, '.cache/project');
	mkdirSync(join(dir, 'src/lib'), { recursive: true });
	mkdirSync(join(dir, 'src/.hidden'), { recursive: true });
	writeFileSync(join(dir, 'src/lib/a.ts'), 'const  a=1\n');
	writeFileSync(join(dir, 'src/.hidden/b.ts'), 'const  b=1\n');
	const result = await format_directory(
		create_mock_logger(),
		join(dir, 'src'),
		true,
		create_empty_gro_config().search_filters,
		undefined,
		dir
	);
	expect(result.formatted).toEqual([join(dir, 'src/lib/a.ts')]);
});

test('format_directory tests glob patterns relative to the root, absolute or from a subdirectory', async () => {
	const dir = join(root, '.cache/project');
	mkdirSync(join(dir, 'src/lib'), { recursive: true });
	mkdirSync(join(dir, 'src/.hidden'), { recursive: true });
	mkdirSync(join(dir, 'node_modules/x'), { recursive: true });
	writeFileSync(join(dir, 'src/lib/a.ts'), 'const  a=1\n');
	writeFileSync(join(dir, 'src/.hidden/b.ts'), 'const  b=1\n');
	writeFileSync(join(dir, 'node_modules/x/c.ts'), 'const  c=1\n');
	const { search_filters } = create_empty_gro_config();
	const format = (patterns: Array<string>) =>
		format_directory(create_mock_logger(), join(dir, 'src'), true, search_filters, patterns, dir);
	const absolute = await format([join(dir, 'src/lib/*.ts'), join(dir, 'src/.hidden/*.ts')]);
	expect(absolute.formatted).toEqual([join(dir, 'src/lib/a.ts')]);
	const cwd = process.cwd();
	try {
		process.chdir(join(dir, 'src/lib'));
		const relative = await format(['../lib/*.ts', '../.hidden/*.ts', '../../node_modules/x/*.ts']);
		expect(relative.formatted).toEqual([join(dir, 'src/lib/a.ts')]);
	} finally {
		process.chdir(cwd);
	}
});

test('format_directory filters relative to the root, whichever directory it sweeps', async () => {
	mkdirSync(join(root, 'src/lib/fixtures'), { recursive: true });
	writeFileSync(join(root, 'src/lib/a.ts'), 'const  a=1\n');
	writeFileSync(join(root, 'src/lib/fixtures/b.ts'), 'const  b=1\n');
	const filter = (p: string) => !p.startsWith('src/lib/fixtures') && !p.startsWith('src/test');
	for (const dir of ['src', 'src/lib']) {
		const result = await format_directory(
			create_mock_logger(),
			join(root, dir),
			true,
			filter,
			undefined,
			root
		);
		expect(result.formatted).toEqual([join(root, 'src/lib/a.ts')]);
	}
});

test('format_directory reads the ignore file from its root', async () => {
	const dir = join(root, 'project');
	mkdirSync(join(dir, 'src/lib'), { recursive: true });
	writeFileSync(join(dir, 'src/lib/a.ts'), 'const  a=1\n');
	writeFileSync(join(dir, 'src/lib/b.ts'), 'const  b=1\n');
	writeFileSync(join(dir, '.formatignore'), 'src/lib/b.ts\n');
	const result = await format_directory(
		create_mock_logger(),
		join(dir, 'src'),
		true,
		create_empty_gro_config().search_filters,
		undefined,
		dir
	);
	expect(result.formatted).toEqual([join(dir, 'src/lib/a.ts')]);
});
