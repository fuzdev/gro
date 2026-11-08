import { describe, test, assert, beforeEach, afterEach } from 'vitest';
import {
	chmodSync,
	existsSync,
	lstatSync,
	mkdirSync,
	readFileSync,
	rmSync,
	symlinkSync,
	writeFileSync
} from 'node:fs';
import { dirname, join, relative } from 'node:path';

import { fs } from '#lib/fs.ts';
import { fs_open_path, fs_remove_path, fs_search_path } from '#lib/internal/fs_path.ts';
import {
	assert_fs_error_rejects,
	create_fs_sandbox,
	remove_fs_sandbox,
	type FsSandbox
} from '../fs_test_helpers.ts';

let sandbox: FsSandbox;

beforeEach(async () => {
	sandbox = await create_fs_sandbox();
});

afterEach(() => {
	remove_fs_sandbox(sandbox);
});

/**
 * A symlink in the project's gitignored `.gro/`, removed after `fn`,
 * for the cases that need a symlink inside the project.
 */
const with_project_link = async (
	target: string,
	fn: (link: string) => Promise<void>
): Promise<void> => {
	const link = join('.gro', `fs_path_test_${process.pid}_${Math.random().toString(36).slice(2)}`);
	mkdirSync(join(fs.project.path, '.gro'), { recursive: true });
	symlinkSync(target, join(fs.project.path, link));
	try {
		await fn(link);
	} finally {
		rmSync(join(fs.project.path, link), { force: true });
	}
};

describe('fs_open_path beneath the project', () => {
	test('a relative path is relative to the project', async () => {
		const opened = await fs_open_path('src/lib/fs.ts');
		assert.strictEqual(opened.dir, fs.project);
		assert.strictEqual(opened.path, 'src/lib/fs.ts');
	});

	test('an absolute path beneath the project is relative to it', async () => {
		const opened = await fs_open_path(join(fs.project.path, 'src/lib/fs.ts'));
		assert.strictEqual(opened.dir, fs.project);
		assert.strictEqual(opened.path, 'src/lib/fs.ts');
	});

	test('the project itself is the empty path, with or without a trailing `/`', async () => {
		for (const path of [fs.project.path, fs.project.path + '/', '.', './']) {
			const opened = await fs_open_path(path);
			assert.strictEqual(opened.dir, fs.project);
			assert.strictEqual(opened.path, '');
		}
	});

	test('keeps a trailing `/` and resolves `.` and `..`', async () => {
		const opened = await fs_open_path(fs.project.path + '/src/./docs/../lib/');
		assert.strictEqual(opened.dir, fs.project);
		assert.strictEqual(opened.path, 'src/lib/');
	});

	test('a missing path keeps its missing part', async () => {
		const opened = await fs_open_path('missing/deeper.ts');
		assert.strictEqual(opened.dir, fs.project);
		assert.strictEqual(opened.path, 'missing/deeper.ts');
	});

	test('a symlink inside the project that leaves it is followed', async () => {
		writeFileSync(join(sandbox.root, 'a.txt'), 'a');
		await with_project_link(sandbox.root, async (link) => {
			const opened = await fs_open_path(link + '/a.txt');
			assert.strictEqual(opened.dir.path, sandbox.root);
			assert.strictEqual(opened.path, 'a.txt');
			assert.strictEqual(await opened.dir.read_text(opened.path), 'a');
		});
	});

	test('a final symlink inside the project that leaves it is followed', async () => {
		writeFileSync(join(sandbox.root, 'a.txt'), 'a');
		await with_project_link(join(sandbox.root, 'a.txt'), async (link) => {
			const opened = await fs_open_path(link);
			assert.strictEqual(opened.dir.path, sandbox.root);
			assert.strictEqual(opened.path, 'a.txt');
			assert.ok(await opened.dir.exists(opened.path));
		});
	});

	test('a dangling symlink inside the project resolves to its missing target', async () => {
		await with_project_link(join(sandbox.root, 'missing.txt'), async (link) => {
			const opened = await fs_open_path(link);
			assert.strictEqual(opened.dir.path, sandbox.root);
			assert.strictEqual(opened.path, 'missing.txt');
			assert.ok(!(await opened.dir.exists(opened.path)));
		});
	});
});

describe('fs_open_path outside the project', () => {
	test('a relative path climbing out of the cwd resolves against it', async () => {
		writeFileSync(join(sandbox.root, 'a.txt'), 'a');
		const path = relative(process.cwd(), join(sandbox.root, 'a.txt'));
		assert.ok(path.startsWith('../'));
		const opened = await fs_open_path(path);
		assert.strictEqual(opened.dir.path, sandbox.root);
		assert.strictEqual(opened.path, 'a.txt');
	});

	test('opens the parent of an existing entry', async () => {
		writeFileSync(join(sandbox.root, 'a.txt'), 'a');
		const opened = await fs_open_path(join(sandbox.root, 'a.txt'));
		assert.strictEqual(opened.dir.path, sandbox.root);
		assert.strictEqual(opened.path, 'a.txt');
		assert.strictEqual(await opened.dir.read_text(opened.path), 'a');
	});

	test('opens the nearest existing ancestor of a missing path', async () => {
		const opened = await fs_open_path(join(sandbox.root, 'x/y/z.txt'));
		assert.strictEqual(opened.dir.path, sandbox.root);
		assert.strictEqual(opened.path, 'x/y/z.txt');
		assert.isNull(await opened.dir.stat(opened.path));
	});

	test('keeps a trailing `/`', async () => {
		mkdirSync(join(sandbox.root, 'sub'));
		const opened = await fs_open_path(join(sandbox.root, 'sub/'));
		assert.strictEqual(opened.dir.path, sandbox.root);
		assert.strictEqual(opened.path, 'sub/');
	});

	test('follows a symlink above the path, wherever it points', async () => {
		symlinkSync(sandbox.outside, join(sandbox.root, 'link'));
		const opened = await fs_open_path(join(sandbox.root, 'link/secret.txt'));
		assert.strictEqual(opened.dir.path, sandbox.outside);
		assert.strictEqual(opened.path, 'secret.txt');
	});

	test('follows a final symlink, absolute or relative', async () => {
		symlinkSync(join(sandbox.outside, 'secret.txt'), join(sandbox.root, 'absolute'));
		symlinkSync('../outside/secret.txt', join(sandbox.root, 'relative'));
		for (const name of ['absolute', 'relative']) {
			const opened = await fs_open_path(join(sandbox.root, name));
			assert.strictEqual(opened.dir.path, sandbox.outside);
			assert.strictEqual(opened.path, 'secret.txt');
		}
	});

	test('follows a chain of dangling symlinks to the missing target', async () => {
		symlinkSync('second', join(sandbox.root, 'first'));
		symlinkSync('../outside/missing/deeper.txt', join(sandbox.root, 'second'));
		const opened = await fs_open_path(join(sandbox.root, 'first'));
		assert.strictEqual(opened.dir.path, sandbox.outside);
		assert.strictEqual(opened.path, 'missing/deeper.txt');
		assert.isNull(await opened.dir.stat(opened.path));
	});

	test('a symlink loop throws `io_error` naming ELOOP', async () => {
		symlinkSync('loop', join(sandbox.root, 'loop'));
		const error = await assert_fs_error_rejects(
			fs_open_path(join(sandbox.root, 'loop')),
			'io_error'
		);
		assert.include(error.message, 'ELOOP');
		const nested = await assert_fs_error_rejects(
			fs_open_path(join(sandbox.root, 'loop/missing')),
			'io_error'
		);
		assert.include(nested.message, 'ELOOP');
	});

	test('an unsearchable directory throws `permission_denied`', async () => {
		mkdirSync(join(sandbox.root, 'locked'));
		chmodSync(join(sandbox.root, 'locked'), 0o000);
		try {
			await assert_fs_error_rejects(
				fs_open_path(join(sandbox.root, 'locked/a.txt')),
				'permission_denied'
			);
		} finally {
			chmodSync(join(sandbox.root, 'locked'), 0o755);
		}
	});

	test('an ancestor that is a file throws `not_a_directory`', async () => {
		writeFileSync(join(sandbox.root, 'a.txt'), 'a');
		await assert_fs_error_rejects(
			fs_open_path(join(sandbox.root, 'a.txt/b.txt')),
			'not_a_directory'
		);
	});
});

describe('fs_search_path', () => {
	beforeEach(() => {
		mkdirSync(join(sandbox.root, 'src/sub'), { recursive: true });
		writeFileSync(join(sandbox.root, 'src/a.ts'), '');
		writeFileSync(join(sandbox.root, 'src/sub/b.ts'), '');
	});

	test('ids start with the path as named, not its real path', async () => {
		symlinkSync('src', join(sandbox.root, 'link'));
		const found = await fs_search_path(join(sandbox.root, 'link'));
		assert.deepEqual(
			found.map((f) => [f.path, f.id]),
			[
				['a.ts', join(sandbox.root, 'link/a.ts')],
				['sub/b.ts', join(sandbox.root, 'link/sub/b.ts')]
			]
		);
	});

	test('passes the filters through', async () => {
		const found = await fs_search_path(join(sandbox.root, 'src'), {
			filter: (id) => !id.endsWith('/sub')
		});
		assert.deepEqual(
			found.map((f) => f.path),
			['a.ts']
		);
	});

	test('a missing directory finds nothing', async () => {
		assert.deepEqual(await fs_search_path(join(sandbox.root, 'missing')), []);
	});

	test('a file throws `not_a_directory`', async () => {
		await assert_fs_error_rejects(
			fs_search_path(join(sandbox.root, 'src/a.ts')),
			'not_a_directory'
		);
	});

	test('searches beneath the project through `fs.project`', async () => {
		const found = await fs_search_path('src/lib', {
			file_filter: (id) => id.endsWith('/fs_search.ts')
		});
		assert.deepEqual(
			found.map((f) => f.id),
			[join(fs.project.path, 'src/lib/fs_search.ts')]
		);
	});
});

describe('fs_remove_path', () => {
	test('removes a final symlink, never what it points at', async () => {
		symlinkSync(join(sandbox.outside, 'secret.txt'), join(sandbox.root, 'link'));
		await fs_remove_path(join(sandbox.root, 'link'));
		assert.ok(!existsSync(join(sandbox.root, 'link')));
		assert.strictEqual(readFileSync(join(sandbox.outside, 'secret.txt'), 'utf8'), 'secret');
	});

	test('follows a symlink above the entry', async () => {
		symlinkSync(sandbox.outside, join(sandbox.root, 'link'));
		await fs_remove_path(join(sandbox.root, 'link/secret.txt'));
		assert.ok(!existsSync(join(sandbox.outside, 'secret.txt')));
		assert.ok(lstatSync(join(sandbox.root, 'link')).isSymbolicLink());
	});

	test('`missing_ok` returns for a missing entry or parent', async () => {
		await fs_remove_path(join(sandbox.root, 'missing.txt'), { missing_ok: true });
		await fs_remove_path(join(sandbox.root, 'missing/deeper.txt'), { missing_ok: true });
		await assert_fs_error_rejects(fs_remove_path(join(sandbox.root, 'missing.txt')), 'not_found');
	});

	test('a trailing `/` removes only a directory', async () => {
		writeFileSync(join(sandbox.root, 'a.txt'), 'a');
		await assert_fs_error_rejects(fs_remove_path(join(sandbox.root, 'a.txt/')), 'not_a_directory');
		assert.ok(existsSync(join(sandbox.root, 'a.txt')));
	});

	// never `recursive` here: a regression would then fail with `not_empty`, deleting nothing
	test('refuses the project and its ancestors', async () => {
		for (const path of ['.', './', 'src/..', '..', process.cwd(), dirname(process.cwd())]) {
			await assert_fs_error_rejects(fs_remove_path(path), 'io_error');
		}
		assert.ok(existsSync(join(process.cwd(), 'package.json')));
	});
});
