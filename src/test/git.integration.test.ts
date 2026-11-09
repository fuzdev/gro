import { describe, test, assert, beforeEach, afterEach } from 'vitest';

import { fs, type FsDir } from '#lib/fs.ts';
import {
	git_check_clean_workspace,
	git_check_setting_pull_rebase,
	git_check_workspace,
	git_checkout,
	git_current_branch_first_commit_hash,
	git_current_branch_name,
	git_current_commit_hash,
	git_delete_local_branch,
	git_fetch,
	git_head_subject,
	git_local_branch_exists,
	git_pull,
	git_push,
	git_push_to_create,
	git_remote_branch_exists,
	git_tag_exists
} from '#lib/git.ts';
import { create_fs_paths } from '#lib/fs_paths.ts';
import { library_cache_key } from '#lib/library_load.ts';
import { spawn, type SpawnProgram } from '#lib/spawn.ts';

/*

Real git, in temporary directories only: a bare repo as the remote, a work repo, and a second
clone of the remote. User config can't leak in, as `GIT_CONFIG_GLOBAL` and `GIT_CONFIG_SYSTEM`
are `/dev/null` and the identity comes from `env`, set when the handle is made.
`GIT_CEILING_DIRECTORIES` stops git's search for a repo at the sandbox, so a directory that's no
repo is one wherever the temporary directory sits, even inside a repo.

*/

const IDENTITY = {
	GIT_CONFIG_GLOBAL: '/dev/null',
	GIT_CONFIG_SYSTEM: '/dev/null',
	GIT_AUTHOR_NAME: 'Fixture',
	GIT_AUTHOR_EMAIL: 'fixture@example.com',
	GIT_COMMITTER_NAME: 'Fixture',
	GIT_COMMITTER_EMAIL: 'fixture@example.com'
};

let base: FsDir & AsyncDisposable;
let git: SpawnProgram;
let work: FsDir;
let remote_path: string;

const run_ok = async (dir: FsDir, args: ReadonlyArray<string>): Promise<string> => {
	const output = await git.out(args, { cwd: dir });
	assert.strictEqual(output.kind, 'exited');
	assert.strictEqual(
		(output as { code: number }).code,
		0,
		`git ${args.join(' ')} failed: ${output.stderr}`
	);
	return output.stdout;
};

const create_git = (extra_env?: Record<string, string>): SpawnProgram =>
	spawn.program('git', {
		env: { ...IDENTITY, GIT_CEILING_DIRECTORIES: base.path, ...extra_env }
	});

const write_commit = async (dir: FsDir, name: string, message: string): Promise<void> => {
	await dir.write(name, message);
	await run_ok(dir, ['add', '--', name]);
	await run_ok(dir, ['commit', '-q', '-m', message]);
};

const to_message = async (promise: Promise<unknown>): Promise<string> => {
	try {
		await promise;
	} catch (error) {
		return (error as Error).message;
	}
	throw Error('expected a rejection');
};

beforeEach(async () => {
	base = await fs.temp_dir('gro-git-integration-');
	git = create_git();
	await base.mkdir('remote.git');
	await base.mkdir('work');
	remote_path = base.path + '/remote.git';
	await run_ok(base.dir('remote.git'), ['init', '-q', '--bare', '-b', 'main']);
	work = base.dir('work');
	await run_ok(work, ['init', '-q', '-b', 'main']);
	await run_ok(work, ['remote', 'add', 'origin', remote_path]);
	await write_commit(work, 'a.txt', 'first');
});

afterEach(async () => {
	await base[Symbol.asyncDispose]();
});

describe('git over a real repo', () => {
	test('reads the branch, the commit, and the first commit', async () => {
		const head = (await run_ok(work, ['rev-parse', 'HEAD'])).trim();
		assert.strictEqual(await git_current_branch_name(git, { cwd: work }), 'main');
		assert.strictEqual(await git_current_commit_hash(git, { cwd: work }), head);
		await write_commit(work, 'b.txt', 'second');
		assert.isTrue(head.startsWith(await git_current_branch_first_commit_hash(git, { cwd: work })));
	});

	test('a directory that is no repo reports no status and no commit', async () => {
		await base.mkdir('plain');
		const plain = base.dir('plain');
		assert.deepEqual(await git_check_workspace(git, { cwd: plain }), {
			unstaged_changes: false,
			staged_changes: false,
			untracked_files: false
		});
		assert.strictEqual(await git_current_commit_hash(git, { cwd: plain }), null);
		assert.include(
			await to_message(git_current_branch_name(git, { cwd: plain })),
			'git_current_branch_name failed'
		);
	});

	test('an unborn branch throws naming the case and the fix', async () => {
		await base.mkdir('unborn');
		const unborn = base.dir('unborn');
		await run_ok(unborn, ['init', '-q', '-b', 'main']);
		const message = await to_message(git_current_branch_name(git, { cwd: unborn }));
		assert.include(message, 'unborn');
		assert.include(message, 'make a first commit');
		assert.strictEqual(await git_current_commit_hash(git, { cwd: unborn }), null);
	});

	test('a detached HEAD reads as the literal HEAD', async () => {
		await run_ok(work, ['checkout', '-q', '--detach']);
		assert.strictEqual(await git_current_branch_name(git, { cwd: work }), 'HEAD');
	});

	test('reads the workspace status', async () => {
		assert.isNull(await git_check_clean_workspace(git, { cwd: work }));
		await work.write('a.txt', 'changed');
		await work.write('new.txt', 'new');
		const status = await git_check_workspace(git, { cwd: work });
		assert.isTrue(status.unstaged_changes);
		assert.isTrue(status.untracked_files);
		assert.isFalse(status.staged_changes);
		assert.include((await git_check_clean_workspace(git, { cwd: work })) ?? '', 'unstaged changes');
	});

	test('git_checkout switches branches and reports the previous one', async () => {
		await run_ok(work, ['branch', 'other']);
		assert.strictEqual(await git_checkout(git, 'other', { cwd: work }), 'main');
		assert.strictEqual(await git_current_branch_name(git, { cwd: work }), 'other');
		assert.isNull(await git_checkout(git, 'other', { cwd: work }));
	});

	test('git_checkout fails with the branch named', async () => {
		assert.include(
			await to_message(git_checkout(git, 'missing', { cwd: work })),
			"git_checkout failed for branch 'missing'"
		);
	});

	test('git_local_branch_exists and git_delete_local_branch', async () => {
		await run_ok(work, ['branch', 'old']);
		assert.isTrue(await git_local_branch_exists(git, 'old', { cwd: work }));
		await git_delete_local_branch(git, 'old', { cwd: work });
		assert.isFalse(await git_local_branch_exists(git, 'old', { cwd: work }));
	});

	test('git_delete_local_branch fails with the branch named', async () => {
		assert.include(
			await to_message(git_delete_local_branch(git, 'missing', { cwd: work })),
			"git_delete_local_branch failed for branch 'missing'"
		);
	});

	test('git_check_setting_pull_rebase reads the global config', async () => {
		assert.isFalse(await git_check_setting_pull_rebase(git, { cwd: work }));
		await base.write('gitconfig', '[pull]\n\trebase = true\n');
		const configured = create_git({ GIT_CONFIG_GLOBAL: base.path + '/gitconfig' });
		assert.isTrue(await git_check_setting_pull_rebase(configured, { cwd: work }));
	});
});

describe('the release commit reads', () => {
	test('`git_head_subject` reads the subject of `HEAD`, with no body', async () => {
		await run_ok(work, ['commit', '-q', '--allow-empty', '-m', 'publish v1.0.0', '-m', 'body']);
		assert.strictEqual(await git_head_subject(git, { cwd: work }), 'publish v1.0.0');
	});

	test('`git_head_subject` is null on an unborn branch', async () => {
		await base.mkdir('unborn');
		const unborn = base.dir('unborn');
		await run_ok(unborn, ['init', '-q', '-b', 'main']);
		assert.isNull(await git_head_subject(git, { cwd: unborn }));
	});

	test('`git_tag_exists` sees a tag and not a branch of that name', async () => {
		await run_ok(work, ['branch', 'v2.0.0']);
		assert.isFalse(await git_tag_exists(git, 'v2.0.0', { cwd: work }));
		await run_ok(work, ['tag', 'v2.0.0', '-m', 'v2.0.0']);
		assert.isTrue(await git_tag_exists(git, 'v2.0.0', { cwd: work }));
	});
});

describe('git_current_commit_hash follows HEAD', () => {
	test('a clone with a new local commit reads that commit, not origin/HEAD', async () => {
		await git_push_to_create(git, 'origin', 'main', { cwd: work });
		await base.mkdir('clone');
		const clone = base.dir('clone');
		await run_ok(clone, ['clone', '-q', remote_path, '.']);
		await write_commit(clone, 'b.txt', 'local');
		const head = (await run_ok(clone, ['rev-parse', 'HEAD'])).trim();
		const origin_head = (await run_ok(clone, ['rev-parse', 'refs/remotes/origin/HEAD'])).trim();
		assert.notStrictEqual(head, origin_head);
		assert.strictEqual(await git_current_commit_hash(git, { cwd: clone }), head);
	});

	test('a branch whose name ends like the current one does not shadow it', async () => {
		const first = (await run_ok(work, ['rev-parse', 'HEAD'])).trim();
		await write_commit(work, 'b.txt', 'second');
		// `a/main` at the older commit, what a tail-matching ref lookup could pick
		await run_ok(work, ['branch', 'a/main', first]);
		const head = (await run_ok(work, ['rev-parse', 'HEAD'])).trim();
		assert.notStrictEqual(head, first);
		assert.strictEqual(await git_current_commit_hash(git, { cwd: work }), head);
	});

	test('a detached HEAD reads as its commit', async () => {
		const first = (await run_ok(work, ['rev-parse', 'HEAD'])).trim();
		await write_commit(work, 'b.txt', 'second');
		await run_ok(work, ['checkout', '-q', first]);
		assert.strictEqual(await git_current_commit_hash(git, { cwd: work }), first);
	});

	test('an unborn branch reads as null, and does not throw', async () => {
		await base.mkdir('unborn');
		const unborn = base.dir('unborn');
		await run_ok(unborn, ['init', '-q', '-b', 'main']);
		assert.strictEqual(await git_current_commit_hash(git, { cwd: unborn }), null);
	});
});

describe('a repo git can not read', () => {
	const assert_status_throws = async (program: SpawnProgram, expected: string): Promise<void> => {
		const message = await to_message(git_check_workspace(program, { cwd: work }));
		assert.include(message, 'git status failed');
		assert.include(message, expected);
	};

	test('a corrupted index throws rather than reading as clean', async () => {
		await work.write('.git/index', 'garbage');
		await assert_status_throws(git, 'index file');
		assert.include(
			await to_message(git_check_clean_workspace(git, { cwd: work })),
			'git status failed'
		);
	});

	test('dubious ownership throws rather than reading as clean', async () => {
		const dubious = create_git({ GIT_TEST_ASSUME_DIFFERENT_OWNER: '1' });
		await assert_status_throws(dubious, 'dubious ownership');
	});

	test('a bad config throws rather than reading as clean', async () => {
		await work.write('.git/config', '[core\nbroken');
		await assert_status_throws(git, 'bad config');
	});

	test('a broken submodule throws rather than reading as not a repo', async () => {
		await run_ok(work, ['-c', 'protocol.file.allow=always', 'submodule', 'add', work.path, 'sub']);
		await work.remove('.git/modules/sub', { recursive: true });
		await assert_status_throws(git, 'not a git repository');
	});

	test('the library cache key is null over a corrupted index, not a thrown error', async () => {
		await work.write('.git/index', 'garbage');
		assert.isNull(await library_cache_key({ fs_paths: create_fs_paths(fs), git }, work.path));
	});
});

describe('git against a local bare remote', () => {
	test('pushes, creating the branch with upstream, then fetches and pulls', async () => {
		assert.isFalse(await git_remote_branch_exists(git, 'origin', 'main', { cwd: work }));
		await git_push_to_create(git, 'origin', 'main', { cwd: work });
		assert.isTrue(await git_remote_branch_exists(git, 'origin', 'main', { cwd: work }));
		assert.strictEqual(
			(await run_ok(work, ['rev-parse', '--abbrev-ref', '--symbolic-full-name', '@{u}'])).trim(),
			'origin/main'
		);

		// a second clone advances the remote, which the first fetches and pulls
		await base.mkdir('other');
		const other = base.dir('other');
		await run_ok(other, ['clone', '-q', remote_path, '.']);
		await write_commit(other, 'b.txt', 'second');
		await git_push(git, 'origin', 'main', { cwd: other });

		await git_fetch(git, 'origin', 'main', { cwd: work });
		assert.isFalse(await work.exists('b.txt'));
		await git_pull(git, 'origin', 'main', { cwd: work });
		assert.isTrue(await work.exists('b.txt'));

		// a repeat push finds the branch and doesn't reset its upstream
		await git_push_to_create(git, 'origin', 'main', { cwd: work });
		assert.isTrue(await git_remote_branch_exists(git, 'origin', 'main', { cwd: work }));
	});

	test('git_fetch fails naming the origin and branch', async () => {
		const message = await to_message(git_fetch(git, 'nope', 'main', { cwd: work }));
		assert.include(message, "git_fetch failed for origin 'nope'");
	});

	test('git_push fails naming the branch', async () => {
		const message = await to_message(git_push(git, 'nope', 'main', { cwd: work }));
		assert.include(message, "git_push failed for branch 'main'");
	});

	test('git_remote_branch_exists throws on a remote it cannot read', async () => {
		const message = await to_message(git_remote_branch_exists(git, 'nope', 'main', { cwd: work }));
		assert.include(message, 'git_remote_branch_exists failed');
	});
});

describe('names beginning with -', () => {
	const MARKER_NAME = 'pwned';

	const assert_no_marker = async (): Promise<void> => {
		assert.isFalse(await base.exists(MARKER_NAME), 'expected git not to run the option');
	};

	test('an origin beginning with - runs nothing in fetch, push, or ls-remote', async () => {
		const marker = base.path + '/' + MARKER_NAME;
		await to_message(git_fetch(git, `--upload-pack=touch ${marker}`, 'main', { cwd: work }));
		await to_message(git_push(git, `--receive-pack=touch ${marker}`, 'main', { cwd: work }));
		await to_message(
			git_remote_branch_exists(git, `--upload-pack=touch ${marker}`, 'main', { cwd: work })
		);
		await assert_no_marker();
	});

	test('git_pull refuses a name beginning with -', async () => {
		const marker = base.path + '/' + MARKER_NAME;
		assert.include(
			await to_message(git_pull(git, `--upload-pack=touch ${marker}`, 'main', { cwd: work })),
			'beginning with -'
		);
		await assert_no_marker();
	});

	test('a branch beginning with - is a failed name, never an option', async () => {
		await run_ok(work, ['branch', 'other']);
		await to_message(git_checkout(git, '--orphan=x', { cwd: work }));
		assert.isFalse(await git_local_branch_exists(git, '--heads', { cwd: work }));
		await to_message(git_delete_local_branch(git, '--force', { cwd: work }));
		assert.strictEqual(await git_current_branch_name(git, { cwd: work }), 'main');
		assert.isTrue(await git_local_branch_exists(git, 'other', { cwd: work }));
	});
});
