import { test, expect, beforeEach, afterEach } from 'vitest';
import {
	lstatSync,
	mkdtempSync,
	mkdirSync,
	readFileSync,
	rmSync,
	symlinkSync,
	writeFileSync
} from 'node:fs';
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

test('no ignore file skips nothing', async () => {
	const is_ignored = await read_format_ignore(root);
	expect(is_ignored(resolve(root, 'notes.md'))).toBe(false);
	expect(is_ignored(resolve(root, 'src/lib/keep.ts'))).toBe(false);
});

test('anchored and non-anchored patterns match absolute ids', async () => {
	writeFileSync(
		join(root, '.formatignore'),
		'# comment\nsrc/test/fixtures/generated/**\nsrc/test/fixtures/samples/sample_*.*\n\n*.md\n'
	);
	const is_ignored = await read_format_ignore(root);
	expect(is_ignored(resolve(root, 'src/test/fixtures/samples/sample_a.ts'))).toBe(true);
	expect(is_ignored(resolve(root, 'src/test/fixtures/samples/sample_b.svelte'))).toBe(true);
	expect(is_ignored(resolve(root, 'src/test/fixtures/generated/out.ts'))).toBe(true);
	// `*.md` has no slash, so it matches at any depth.
	expect(is_ignored(resolve(root, 'notes.md'))).toBe(true);
	expect(is_ignored(resolve(root, 'src/lib/notes.md'))).toBe(true);
	// unlisted source is not ignored.
	expect(is_ignored(resolve(root, 'src/lib/keep.ts'))).toBe(false);
});

test('a directory pattern skips everything beneath it, and a negation re-includes', async () => {
	writeFileSync(join(root, '.formatignore'), 'fixtures/\n*.ts\n!keep.ts\n');
	const is_ignored = await read_format_ignore(root);
	expect(is_ignored(resolve(root, 'src/test/fixtures/samples/sample_b.svelte'))).toBe(true);
	expect(is_ignored(resolve(root, 'src/lib/other.ts'))).toBe(true);
	expect(is_ignored(resolve(root, 'src/lib/keep.ts'))).toBe(false);
});

test('a file outside the root is never ignored', async () => {
	writeFileSync(join(root, '.formatignore'), '*.ts\n');
	const is_ignored = await read_format_ignore(join(root, 'src'));
	expect(is_ignored(resolve(root, 'outside.ts'))).toBe(false);
});

test('a blank/comment-only .formatignore skips nothing', async () => {
	writeFileSync(join(root, '.formatignore'), '# only a comment\n\n');
	const is_ignored = await read_format_ignore(root);
	expect(is_ignored(resolve(root, 'notes.md'))).toBe(false);
});

test('a .formatignore shadows a sibling .prettierignore, as tsv reads them', async () => {
	writeFileSync(join(root, '.formatignore'), 'src/test/fixtures/samples/sample_*.*\n');
	writeFileSync(join(root, '.prettierignore'), 'src/test/fixtures/generated/**\n');
	const is_ignored = await read_format_ignore(root);
	// from .formatignore
	expect(is_ignored(resolve(root, 'src/test/fixtures/samples/sample_a.ts'))).toBe(true);
	// the shadowed .prettierignore is never read
	expect(is_ignored(resolve(root, 'src/test/fixtures/generated/out.ts'))).toBe(false);
	expect(is_ignored(resolve(root, 'src/lib/keep.ts'))).toBe(false);
});

test('.prettierignore alone (no .formatignore) is respected', async () => {
	writeFileSync(join(root, '.prettierignore'), 'src/test/fixtures/samples/sample_*.*\n');
	const is_ignored = await read_format_ignore(root);
	expect(is_ignored(resolve(root, 'src/test/fixtures/samples/sample_a.ts'))).toBe(true);
	expect(is_ignored(resolve(root, 'src/lib/keep.ts'))).toBe(false);
});

test('format_directory searches a project under a dot-directory', async () => {
	const dir = join(root, '.cache/project');
	mkdirSync(join(dir, 'src/lib'), { recursive: true });
	mkdirSync(join(dir, 'src/.hidden'), { recursive: true });
	writeFileSync(join(dir, 'src/lib/a.ts'), 'const  a=1\n');
	writeFileSync(join(dir, 'src/.hidden/b.ts'), 'const  b=1\n');
	const result = await format_directory({
		log: create_mock_logger(),
		dir: join(dir, 'src'),
		check: true,
		filter: create_empty_gro_config().search_filters,
		root: dir
	});
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
		format_directory({
			log: create_mock_logger(),
			dir: join(dir, 'src'),
			check: true,
			filter: search_filters,
			patterns,
			root: dir
		});
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

test('format_directory finds files outside the root under a dot-directory, swept or globbed', async () => {
	const dir = join(root, 'project');
	const other = join(root, '.cache/other');
	mkdirSync(join(dir, 'src/lib'), { recursive: true });
	mkdirSync(join(other, 'src/.hidden'), { recursive: true });
	mkdirSync(join(other, 'src/lib'), { recursive: true });
	writeFileSync(join(other, 'src/lib/a.ts'), 'const  a=1\n');
	writeFileSync(join(other, 'src/.hidden/b.ts'), 'const  b=1\n');
	const { search_filters } = create_empty_gro_config();
	const swept = await format_directory({
		log: create_mock_logger(),
		dir: join(other, 'src'),
		check: true,
		filter: search_filters,
		root: dir
	});
	expect(swept.formatted).toEqual([join(other, 'src/lib/a.ts')]);
	const format = (patterns: Array<string>) =>
		format_directory({
			log: create_mock_logger(),
			dir: join(dir, 'src'),
			check: true,
			filter: search_filters,
			patterns,
			root: dir
		});
	const cwd = process.cwd();
	try {
		// from a cwd under a dot-directory outside both, which an entry's bare name never resolves to
		const elsewhere = join(root, '.elsewhere');
		mkdirSync(elsewhere);
		process.chdir(elsewhere);
		const absolute = await format([join(other, 'src/**/*.ts')]);
		expect(absolute.formatted).toEqual([join(other, 'src/lib/a.ts')]);
		process.chdir(dir);
		const relative = await format(['../.cache/other/src/**/*.ts']);
		expect(relative.formatted).toEqual([join(other, 'src/lib/a.ts')]);
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
		const result = await format_directory({
			log: create_mock_logger(),
			dir: join(root, dir),
			check: true,
			filter,
			root
		});
		expect(result.formatted).toEqual([join(root, 'src/lib/a.ts')]);
	}
});

test('format_directory reads the ignore file from its root', async () => {
	const dir = join(root, 'project');
	mkdirSync(join(dir, 'src/lib'), { recursive: true });
	writeFileSync(join(dir, 'src/lib/a.ts'), 'const  a=1\n');
	writeFileSync(join(dir, 'src/lib/b.ts'), 'const  b=1\n');
	writeFileSync(join(dir, '.formatignore'), 'src/lib/b.ts\n');
	const result = await format_directory({
		log: create_mock_logger(),
		dir: join(dir, 'src'),
		check: true,
		filter: create_empty_gro_config().search_filters,
		root: dir
	});
	expect(result.formatted).toEqual([join(dir, 'src/lib/a.ts')]);
});

test('format_directory matches a glob without `**` only at its depth', async () => {
	writeFileSync(join(root, 'src/a.ts'), 'const  a=1\n');
	writeFileSync(join(root, 'src/lib/b.ts'), 'const  b=1\n');
	const format = (patterns: Array<string>) =>
		format_directory({
			log: create_mock_logger(),
			dir: join(root, 'src'),
			check: true,
			patterns,
			root
		});
	expect((await format([join(root, 'src/*.ts')])).formatted).toEqual([join(root, 'src/a.ts')]);
	expect((await format([join(root, 'src/*/b.ts')])).formatted).toEqual([
		join(root, 'src/lib/b.ts')
	]);
	expect((await format([join(root, 'src/**/*.ts')])).formatted).toEqual([
		join(root, 'src/a.ts'),
		join(root, 'src/lib/b.ts')
	]);
});

test('a sweep skips symlinks and acts on regular files only', async () => {
	mkdirSync(join(root, 'src/lib/real'));
	writeFileSync(join(root, 'src/lib/real/real.ts'), 'const  r=1\n');
	writeFileSync(join(root, 'outside.ts'), 'const  o=1\n');
	symlinkSync('real/real.ts', join(root, 'src/lib/relative.ts'));
	symlinkSync(join(root, 'outside.ts'), join(root, 'src/lib/absolute.ts'));
	const format = (patterns?: Array<string>) =>
		format_directory({ log: create_mock_logger(), dir: join(root, 'src/lib'), patterns, root });
	const swept = await format();
	expect(swept.ok).toBe(true);
	expect(swept.formatted).toEqual([join(root, 'src/lib/real/real.ts')]);
	const globbed = await format([join(root, 'src/lib/*.ts')]);
	expect(globbed.formatted).toEqual([]);
	expect(lstatSync(join(root, 'src/lib/relative.ts')).isSymbolicLink()).toBe(true);
	expect(readFileSync(join(root, 'outside.ts'), 'utf8')).toBe('const  o=1\n');
});

test('a file named explicitly is followed through a symlink', async () => {
	writeFileSync(join(root, 'outside.ts'), 'const  o=1\n');
	symlinkSync(join(root, 'outside.ts'), join(root, 'src/lib/absolute.ts'));
	const result = await format_directory({
		log: create_mock_logger(),
		dir: join(root, 'src'),
		patterns: [join(root, 'src/lib/absolute.ts')],
		root
	});
	expect(result.formatted).toEqual([join(root, 'src/lib/absolute.ts')]);
	expect(readFileSync(join(root, 'outside.ts'), 'utf8')).toBe('const o = 1;\n');
	expect(lstatSync(join(root, 'src/lib/absolute.ts')).isSymbolicLink()).toBe(true);
});

test('a root file that is a symlink is skipped, never followed', async () => {
	writeFileSync(join(root, 'outside.ts'), 'const  o=1\n');
	writeFileSync(join(root, 'vite.config.ts'), 'const  v=1\n');
	symlinkSync(join(root, 'outside.ts'), join(root, 'gro.config.ts'));
	const result = await format_directory({
		log: create_mock_logger(),
		dir: join(root, 'src/'),
		root
	});
	expect(result.ok).toBe(true);
	expect(result.formatted).toContain(join(root, 'vite.config.ts'));
	expect(result.formatted).not.toContain(join(root, 'gro.config.ts'));
	expect(readFileSync(join(root, 'outside.ts'), 'utf8')).toBe('const  o=1\n');
	expect(lstatSync(join(root, 'gro.config.ts')).isSymbolicLink()).toBe(true);
});

test('an existing file is never a glob, even with glob syntax in its name', async () => {
	mkdirSync(join(root, 'src/routes/[slug]'), { recursive: true });
	mkdirSync(join(root, 'src/routes/s'), { recursive: true });
	writeFileSync(join(root, 'src/routes/[slug]/+page.ts'), 'const  p=1\n');
	// what `[slug]` would match as a character class
	writeFileSync(join(root, 'src/routes/s/+page.ts'), 'const  s=1\n');
	const format = (patterns: Array<string>) =>
		format_directory({
			log: create_mock_logger(),
			dir: join(root, 'src'),
			check: true,
			patterns,
			root
		});
	// as lint-staged passes them, absolute
	expect((await format([join(root, 'src/routes/[slug]/+page.ts')])).formatted).toEqual([
		join(root, 'src/routes/[slug]/+page.ts')
	]);
	const cwd = process.cwd();
	try {
		process.chdir(root);
		expect((await format(['src/routes/[slug]/+page.ts'])).formatted).toEqual([
			join(root, 'src/routes/[slug]/+page.ts')
		]);
		// a missing one is a glob
		expect((await format(['src/routes/[s]/+page.ts'])).formatted).toEqual([
			join(root, 'src/routes/s/+page.ts')
		]);
		// a missing path with no glob syntax names nothing
		expect((await format(['src/routes/missing.ts'])).formatted).toEqual([]);
	} finally {
		process.chdir(cwd);
	}
});
