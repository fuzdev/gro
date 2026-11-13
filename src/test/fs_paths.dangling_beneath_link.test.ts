import { describe, test, assert, afterEach } from 'vitest';
import { mkdirSync, realpathSync, symlinkSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { create_project_fs_paths } from './fs_test_helpers.ts';

let base: string | undefined;

afterEach(() => {
	if (base) rmSync(base, { recursive: true, force: true });
	base = undefined;
});

describe('a dangling symlink beneath a symlinked directory', () => {
	test('its relative target resolves against the real directory, not the link', async () => {
		base = realpathSync(mkdtempSync(join(tmpdir(), 'gro-dangling-')));
		mkdirSync(join(base, 'project'));
		mkdirSync(join(base, 'deep/real'), { recursive: true });
		symlinkSync(join(base, 'deep/real'), join(base, 'project/to_real'));
		// `..` from the real directory is `deep`, from the link's lexical parent the project
		symlinkSync('../gone', join(base, 'deep/real/dangling'));
		const fs_paths = await create_project_fs_paths(join(base, 'project'));
		const opened = await fs_paths.open_path(join(base, 'project/to_real/dangling'));
		assert.strictEqual(opened.dir.path, join(base, 'deep'));
		assert.strictEqual(opened.path, 'gone');
	});
});
