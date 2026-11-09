import { describe, test, assert, beforeEach, afterEach } from 'vitest';

import { changeset_has_pending } from '#lib/changeset_helpers.ts';

import { create_fs_sandbox, remove_fs_sandbox, type FsSandbox } from './fs_test_helpers.ts';

describe('changeset_has_pending', () => {
	let sandbox: FsSandbox;

	beforeEach(async () => {
		sandbox = await create_fs_sandbox();
	});

	afterEach(() => {
		remove_fs_sandbox(sandbox);
	});

	test('is false with no `.changeset` directory', async () => {
		assert.isFalse(await changeset_has_pending(sandbox.dir));
	});

	test('is false with only the README and the config', async () => {
		await sandbox.dir.mkdir('.changeset');
		await sandbox.dir.write('.changeset/README.md', 'docs');
		await sandbox.dir.write('.changeset/config.json', '{}');
		assert.isFalse(await changeset_has_pending(sandbox.dir));
	});

	test('is true with a changeset', async () => {
		await sandbox.dir.mkdir('.changeset');
		await sandbox.dir.write('.changeset/README.md', 'docs');
		await sandbox.dir.write('.changeset/some-change.md', '---\n---\n');
		assert.isTrue(await changeset_has_pending(sandbox.dir));
	});

	test('a directory named like a changeset is not one', async () => {
		await sandbox.dir.mkdir('.changeset/dir.md', { recursive: true });
		assert.isFalse(await changeset_has_pending(sandbox.dir));
	});
});
