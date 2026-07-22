import { test, expect, beforeEach, afterEach } from 'vitest';
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { read_format_ignore } from '$lib/format_directory.ts';

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
