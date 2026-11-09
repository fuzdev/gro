import { describe, test, assert } from 'vitest';
import * as s from '@fuzdev/fuz_schema/schema.ts';

import {
	GIT_LOCAL_PASS_ENV,
	git_parse_workspace_status,
	git_workspace_status_message,
	git_workspace_is_clean,
	git_workspace_is_fully_staged,
	GitBranch,
	GitOrigin,
	type GitWorkspaceStatus
} from '#lib/git.ts';

describe('git_workspace_is_clean', () => {
	test('returns true when all flags are false', () => {
		const clean_status: GitWorkspaceStatus = {
			unstaged_changes: false,
			staged_changes: false,
			untracked_files: false
		};
		assert.strictEqual(git_workspace_is_clean(clean_status), true);
	});

	test('returns false when any flag is true', () => {
		assert.strictEqual(
			git_workspace_is_clean({
				unstaged_changes: true,
				staged_changes: false,
				untracked_files: false
			}),
			false
		);
		assert.strictEqual(
			git_workspace_is_clean({
				unstaged_changes: false,
				staged_changes: true,
				untracked_files: false
			}),
			false
		);
		assert.strictEqual(
			git_workspace_is_clean({
				unstaged_changes: false,
				staged_changes: false,
				untracked_files: true
			}),
			false
		);
	});
});

describe('git_workspace_is_fully_staged', () => {
	test('returns true when only staged changes exist', () => {
		const staged_status: GitWorkspaceStatus = {
			unstaged_changes: false,
			staged_changes: true,
			untracked_files: false
		};
		assert.strictEqual(git_workspace_is_fully_staged(staged_status), true);
	});

	test('returns true when workspace is clean', () => {
		const clean_status: GitWorkspaceStatus = {
			unstaged_changes: false,
			staged_changes: false,
			untracked_files: false
		};
		assert.strictEqual(git_workspace_is_fully_staged(clean_status), true);
	});

	test('returns false when unstaged changes exist', () => {
		assert.strictEqual(
			git_workspace_is_fully_staged({
				unstaged_changes: true,
				staged_changes: false,
				untracked_files: false
			}),
			false
		);
	});

	test('returns false when untracked files exist', () => {
		assert.strictEqual(
			git_workspace_is_fully_staged({
				unstaged_changes: false,
				staged_changes: false,
				untracked_files: true
			}),
			false
		);
	});
});

describe('git_workspace_status_message', () => {
	test('returns clean message when workspace is clean', () => {
		const clean_status: GitWorkspaceStatus = {
			unstaged_changes: false,
			staged_changes: false,
			untracked_files: false
		};
		assert.strictEqual(git_workspace_status_message(clean_status), 'workspace is clean');
	});

	test('lists all issues when multiple flags are true', () => {
		const dirty_status: GitWorkspaceStatus = {
			unstaged_changes: true,
			staged_changes: true,
			untracked_files: true
		};
		const message = git_workspace_status_message(dirty_status);
		assert.ok(message.includes('unstaged changes'));
		assert.ok(message.includes('staged but uncommitted changes'));
		assert.ok(message.includes('untracked files'));
	});

	test('message for unstaged changes only', () => {
		const message = git_workspace_status_message({
			unstaged_changes: true,
			staged_changes: false,
			untracked_files: false
		});
		assert.strictEqual(message, 'git has unstaged changes');
	});

	test('message for staged changes only', () => {
		const message = git_workspace_status_message({
			unstaged_changes: false,
			staged_changes: true,
			untracked_files: false
		});
		assert.strictEqual(message, 'git has staged but uncommitted changes');
	});

	test('message for untracked files only', () => {
		const message = git_workspace_status_message({
			unstaged_changes: false,
			staged_changes: false,
			untracked_files: true
		});
		assert.strictEqual(message, 'git has untracked files');
	});
});

// Integration tests - these actually call git

describe('git_parse_workspace_status', () => {
	test('empty output returns clean status', () => {
		const status = git_parse_workspace_status('');
		assert.deepEqual(status, {
			unstaged_changes: false,
			staged_changes: false,
			untracked_files: false
		});
	});

	test('null output returns clean status', () => {
		const status = git_parse_workspace_status(null);
		assert.deepEqual(status, {
			unstaged_changes: false,
			staged_changes: false,
			untracked_files: false
		});
	});

	test('unstaged changes only ( M)', () => {
		const status = git_parse_workspace_status(' M src/lib/git.ts\0');
		assert.deepEqual(status, {
			unstaged_changes: true,
			staged_changes: false,
			untracked_files: false
		});
	});

	test('staged changes only (M )', () => {
		const status = git_parse_workspace_status('M  src/lib/path.test.ts\0');
		assert.deepEqual(status, {
			unstaged_changes: false,
			staged_changes: true,
			untracked_files: false
		});
	});

	test('both staged and unstaged (MM)', () => {
		const status = git_parse_workspace_status('MM src/lib/git.ts\0');
		assert.deepEqual(status, {
			unstaged_changes: true,
			staged_changes: true,
			untracked_files: false
		});
	});

	test('untracked files (??)', () => {
		const status = git_parse_workspace_status('?? foo\0');
		assert.deepEqual(status, {
			unstaged_changes: false,
			staged_changes: false,
			untracked_files: true
		});
	});

	test('ignored files (!!) are treated as no change', () => {
		const status = git_parse_workspace_status('!! ignored.tmp\0');
		assert.deepEqual(status, {
			unstaged_changes: false,
			staged_changes: false,
			untracked_files: false
		});
	});

	test('multiple files with mixed states', () => {
		const porcelain = 'MM src/lib/git.ts\0M  src/lib/path.test.ts\0?? foo\0';
		const status = git_parse_workspace_status(porcelain);
		assert.deepEqual(status, {
			unstaged_changes: true,
			staged_changes: true,
			untracked_files: true
		});
	});

	test('added file (A )', () => {
		const status = git_parse_workspace_status('A  new-file.ts\0');
		assert.deepEqual(status, {
			unstaged_changes: false,
			staged_changes: true,
			untracked_files: false
		});
	});

	test('deleted from index (D )', () => {
		const status = git_parse_workspace_status('D  removed.ts\0');
		assert.deepEqual(status, {
			unstaged_changes: false,
			staged_changes: true,
			untracked_files: false
		});
	});

	test('deleted in working tree ( D)', () => {
		const status = git_parse_workspace_status(' D deleted-file.ts\0');
		assert.deepEqual(status, {
			unstaged_changes: true,
			staged_changes: false,
			untracked_files: false
		});
	});

	test('renamed in index (R )', () => {
		const status = git_parse_workspace_status('R  new.ts\0old.ts\0');
		assert.deepEqual(status, {
			unstaged_changes: false,
			staged_changes: true,
			untracked_files: false
		});
	});

	test('type changed in index (T )', () => {
		const status = git_parse_workspace_status('T  symlink.txt\0');
		assert.deepEqual(status, {
			unstaged_changes: false,
			staged_changes: true,
			untracked_files: false
		});
	});

	test('type changed in working tree ( T)', () => {
		const status = git_parse_workspace_status(' T file.txt\0');
		assert.deepEqual(status, {
			unstaged_changes: true,
			staged_changes: false,
			untracked_files: false
		});
	});

	test('copied in index (C )', () => {
		const status = git_parse_workspace_status('C  copy.ts\0original.ts\0');
		assert.deepEqual(status, {
			unstaged_changes: false,
			staged_changes: true,
			untracked_files: false
		});
	});

	test('unmerged (UU)', () => {
		const status = git_parse_workspace_status('UU conflicted.ts\0');
		assert.deepEqual(status, {
			unstaged_changes: true,
			staged_changes: true,
			untracked_files: false
		});
	});

	test('unmerged, both deleted (DD)', () => {
		const status = git_parse_workspace_status('DD deleted-by-both.ts\0');
		assert.deepEqual(status, {
			unstaged_changes: true,
			staged_changes: true,
			untracked_files: false
		});
	});

	test('unmerged, added by us (AU)', () => {
		const status = git_parse_workspace_status('AU added-by-us.ts\0');
		assert.deepEqual(status, {
			unstaged_changes: true,
			staged_changes: true,
			untracked_files: false
		});
	});

	test('unmerged, deleted by them (UD)', () => {
		const status = git_parse_workspace_status('UD deleted-by-them.ts\0');
		assert.deepEqual(status, {
			unstaged_changes: true,
			staged_changes: true,
			untracked_files: false
		});
	});

	test('unmerged, added by them (UA)', () => {
		const status = git_parse_workspace_status('UA added-by-them.ts\0');
		assert.deepEqual(status, {
			unstaged_changes: true,
			staged_changes: true,
			untracked_files: false
		});
	});

	test('unmerged, deleted by us (DU)', () => {
		const status = git_parse_workspace_status('DU deleted-by-us.ts\0');
		assert.deepEqual(status, {
			unstaged_changes: true,
			staged_changes: true,
			untracked_files: false
		});
	});

	test('unmerged, both added (AA)', () => {
		const status = git_parse_workspace_status('AA both-added.ts\0');
		assert.deepEqual(status, {
			unstaged_changes: true,
			staged_changes: true,
			untracked_files: false
		});
	});

	test('renamed in index, modified in work tree (RM)', () => {
		const status = git_parse_workspace_status('RM new.ts\0old.ts\0');
		assert.deepEqual(status, {
			unstaged_changes: true,
			staged_changes: true,
			untracked_files: false
		});
	});

	test('added in index, modified in work tree (AM)', () => {
		const status = git_parse_workspace_status('AM newly-added.ts\0');
		assert.deepEqual(status, {
			unstaged_changes: true,
			staged_changes: true,
			untracked_files: false
		});
	});

	test('added in index, deleted in work tree (AD)', () => {
		const status = git_parse_workspace_status('AD added-then-deleted.ts\0');
		assert.deepEqual(status, {
			unstaged_changes: true,
			staged_changes: true,
			untracked_files: false
		});
	});

	test('renamed in work tree ( R)', () => {
		// Work tree rename (only possible with certain git configurations)
		const status = git_parse_workspace_status(' R new-work.ts\0old-work.ts\0');
		assert.deepEqual(status, {
			unstaged_changes: true,
			staged_changes: false,
			untracked_files: false
		});
	});

	test('copied in work tree ( C)', () => {
		// Work tree copy (only possible with certain git configurations)
		const status = git_parse_workspace_status(' C copy-work.ts\0original-work.ts\0');
		assert.deepEqual(status, {
			unstaged_changes: true,
			staged_changes: false,
			untracked_files: false
		});
	});

	test('handles trailing NUL correctly', () => {
		// With -z format, trailing NUL is normal and handled by filter(Boolean)
		const status = git_parse_workspace_status(' M src/lib/git.ts\0');
		assert.deepEqual(status, {
			unstaged_changes: true,
			staged_changes: false,
			untracked_files: false
		});
	});

	test('handles empty entries from multiple NULs', () => {
		// Multiple NULs create empty strings that are filtered out
		const status = git_parse_workspace_status(' M src/lib/git.ts\0\0\0');
		assert.deepEqual(status, {
			unstaged_changes: true,
			staged_changes: false,
			untracked_files: false
		});
	});

	test('handles files with spaces in name', () => {
		// With -z format, spaces are preserved literally without quotes
		const status = git_parse_workspace_status('M  file with spaces.ts\0');
		assert.deepEqual(status, {
			unstaged_changes: false,
			staged_changes: true,
			untracked_files: false
		});
	});

	test('handles files with newlines in name', () => {
		// With -z format, newlines are preserved literally
		const status = git_parse_workspace_status('M  file\nwith\nnewlines.ts\0');
		assert.deepEqual(status, {
			unstaged_changes: false,
			staged_changes: true,
			untracked_files: false
		});
	});

	test('handles files with quotes in name', () => {
		// With -z format, quotes are preserved literally
		const status = git_parse_workspace_status('M  file"with"quotes.ts\0');
		assert.deepEqual(status, {
			unstaged_changes: false,
			staged_changes: true,
			untracked_files: false
		});
	});

	test('handles very long file paths', () => {
		const longPath = 'a/'.repeat(100) + 'file.ts';
		const status = git_parse_workspace_status(`M  ${longPath}\0`);
		assert.deepEqual(status, {
			unstaged_changes: false,
			staged_changes: true,
			untracked_files: false
		});
	});

	test('handles renamed file with spaces', () => {
		// Renamed files with spaces: R  new name\0old name\0
		const status = git_parse_workspace_status('R  new file name.ts\0old file name.ts\0');
		assert.deepEqual(status, {
			unstaged_changes: false,
			staged_changes: true,
			untracked_files: false
		});
	});

	test('handles multiple renames and copies in single output', () => {
		// Multiple rename/copy operations mixed with other changes
		const porcelain =
			'R  renamed1.ts\0old1.ts\0' + // Rename in index
			'C  copied.ts\0original.ts\0' + // Copy in index
			'M  modified.ts\0' + // Regular modified file
			'RM renamed2.ts\0old2.ts\0' + // Renamed and modified
			'?? untracked.ts\0'; // Untracked file
		const status = git_parse_workspace_status(porcelain);
		assert.deepEqual(status, {
			unstaged_changes: true, // From RM
			staged_changes: true, // From R, C, M, and RM
			untracked_files: true // From ??
		});
	});
});

describe('GIT_LOCAL_PASS_ENV', () => {
	test('is frozen, so importing it grants no way to widen what later programs pass', () => {
		assert.ok(Object.isFrozen(GIT_LOCAL_PASS_ENV));
	});
});

describe('GitOrigin and GitBranch', () => {
	test('parse a string as it is', () => {
		const origin = s.parse(GitOrigin, 'origin');
		const branch = s.parse(GitBranch, 'main');
		assert.deepEqual(origin, { ok: true, value: 'origin' });
		assert.deepEqual(branch, { ok: true, value: 'main' });
	});

	test('refuse a non-string', () => {
		const result = s.parse(GitBranch, 1);
		assert.strictEqual(result.ok, false);
		if (!result.ok) {
			assert.strictEqual(result.issues[0]?.kind, 'string');
		}
	});
});
