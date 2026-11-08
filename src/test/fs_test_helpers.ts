import { assert } from 'vitest';
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { fs, is_fs_error, type FsDir, type FsError, type FsErrorKind } from '#lib/fs.ts';

/**
 * A real directory tree for fs tests: `root` is the handle's directory,
 * and `outside` a sibling holding `secret.txt`, which no op on `root` may reach.
 */
export interface FsSandbox {
	base: string;
	root: string;
	outside: string;
	dir: FsDir;
}

export const create_fs_sandbox = async (): Promise<FsSandbox> => {
	const base = realpathSync(mkdtempSync(join(tmpdir(), 'gro-fs-test-')));
	const root = join(base, 'root');
	const outside = join(base, 'outside');
	mkdirSync(root);
	mkdirSync(outside);
	writeFileSync(join(outside, 'secret.txt'), 'secret');
	return { base, root, outside, dir: await fs.open_dir(root) };
};

export const remove_fs_sandbox = (sandbox: FsSandbox): void => {
	rmSync(sandbox.base, { recursive: true, force: true });
};

export const assert_fs_error_rejects = async (
	promise: Promise<unknown>,
	kind: FsErrorKind
): Promise<FsError> => {
	try {
		await promise;
	} catch (error) {
		assert.ok(is_fs_error(error, kind), `expected an FsError of kind ${kind}, got ${error}`);
		return error;
	}
	assert.fail(`expected an FsError of kind ${kind}, but it resolved`);
};

export const assert_fs_error_throws = (fn: () => unknown, kind: FsErrorKind): FsError => {
	try {
		fn();
	} catch (error) {
		assert.ok(is_fs_error(error, kind), `expected an FsError of kind ${kind}, got ${error}`);
		return error;
	}
	assert.fail(`expected an FsError of kind ${kind}, but it returned`);
};
