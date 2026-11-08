import { describe, test, assert, vi, beforeEach, afterEach } from 'vitest';
import {
	chmodSync,
	existsSync,
	lstatSync,
	mkdirSync,
	readdirSync,
	readFileSync,
	rmSync,
	symlinkSync,
	writeFileSync
} from 'node:fs';
import { dirname, join, relative } from 'node:path';

import type { FsPaths } from '#lib/fs_paths.ts';

import {
	assert_fs_error_rejects,
	create_fs_sandbox,
	remove_fs_sandbox,
	type FsSandbox
} from '../fs_test_helpers.ts';

/*

The boundary's resolution, through `create_fs_paths(fs)`.
Hermetic: the cwd moves to a sandbox project and the modules reload,
so `fs.project` is the sandbox, and no destructive op can reach the repo.
`process.chdir` needs Vitest's default `forks` pool.

*/

let sandbox: FsSandbox;
let project: string;
let cwd: string;
let fs: (typeof import('#lib/fs.ts'))['fs'];
let fs_copy_path: FsPaths['copy_path'];
let fs_empty_dir_path: FsPaths['empty_dir_path'];
let fs_open_dir_path: FsPaths['open_dir_path'];
let fs_open_path: FsPaths['open_path'];
let fs_remove_path: FsPaths['remove_path'];
let fs_search_path: FsPaths['search_path'];
let fs_protected_dirs: () => Promise<Array<string>>;
let is_protected_dir: (typeof import('#lib/internal/fs_boundary.ts'))['is_protected_dir'];

beforeEach(async () => {
	sandbox = await create_fs_sandbox();
	project = join(sandbox.base, 'project');
	mkdirSync(join(project, 'src/lib'), { recursive: true });
	mkdirSync(join(project, 'src/docs'));
	writeFileSync(join(project, 'package.json'), '{}');
	writeFileSync(join(project, 'src/lib/fs.ts'), '');
	writeFileSync(join(project, 'src/lib/fs_search.ts'), '');
	cwd = process.cwd();
	process.chdir(project);
	vi.resetModules();
	({ fs } = await import('#lib/fs.ts'));
	const { create_fs_paths } = await import('#lib/fs_paths.ts');
	({
		copy_path: fs_copy_path,
		empty_dir_path: fs_empty_dir_path,
		open_dir_path: fs_open_dir_path,
		open_path: fs_open_path,
		remove_path: fs_remove_path,
		search_path: fs_search_path
	} = create_fs_paths(fs));
	const { to_protected_dirs } = await import('#lib/internal/fs_boundary.ts');
	fs_protected_dirs = () => to_protected_dirs(fs.project.path);
	({ is_protected_dir } = await import('#lib/internal/fs_boundary.ts'));
	assert.strictEqual(fs.project.path, project);
});

afterEach(() => {
	process.chdir(cwd);
	vi.resetModules();
	remove_fs_sandbox(sandbox);
});

describe('is_protected_dir', () => {
	const p = '/home/desk/dev/gro';
	const home = '/home/desk';

	test('the root, the project, and each ancestor', () => {
		for (const path of ['/', '/home', '/home/desk', '/home/desk/dev', p]) {
			assert.ok(is_protected_dir(path, [p]), path);
		}
	});

	test('ignores trailing slashes on either', () => {
		for (const path of ['//', '/home/', '/home/desk/dev/', p + '/', p + '//']) {
			assert.ok(is_protected_dir(path, [p]), path);
			assert.ok(is_protected_dir(path, [p + '/']), path);
		}
	});

	test('a path beneath the project, a sibling, or a prefix is neither', () => {
		for (const path of [
			p + '/.gro',
			p + '/src/lib',
			'/home/desk/dev/gro2',
			'/home/desk/dev/gr',
			'/home/desk/de',
			'/home/desk/dev/other',
			'/tmp'
		]) {
			assert.ok(!is_protected_dir(path, [p]), path);
		}
	});

	test('with the project at the root, only the root', () => {
		assert.ok(is_protected_dir('/', ['/']));
		assert.ok(!is_protected_dir('/home', ['/']));
	});

	test('`$HOME` and its ancestors, beside a project outside it', () => {
		const project = '/srv/project';
		for (const path of ['/', '/home', home, home + '/', '/srv', project]) {
			assert.ok(is_protected_dir(path, [project, home]), path);
		}
		for (const path of [home + '/docs', home + '/dev', '/homes', '/home/des', '/srv/other']) {
			assert.ok(!is_protected_dir(path, [project, home]), path);
		}
	});

	test('`to_protected_dirs` holds the project', async () => {
		const dirs = await fs_protected_dirs();
		assert.include(dirs, project);
		assert.ok(is_protected_dir(project, dirs));
		assert.ok(is_protected_dir(dirname(project), dirs));
		assert.ok(!is_protected_dir(join(project, '.gro'), dirs));
	});
});

/**
 * A symlink in the sandbox project's `.gro/`, removed after `fn`,
 * for the cases that need a symlink inside the project.
 */
const with_project_link = async (
	target: string,
	fn: (link: string) => Promise<void>
): Promise<void> => {
	const link = join(
		'.gro',
		`fs_boundary_test_${process.pid}_${Math.random().toString(36).slice(2)}`
	);
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

describe('fs_open_path and symlink targets that are not UTF-8', () => {
	test('a dangling symlink whose target is not UTF-8 is refused, never a lossy path', async () => {
		symlinkSync(Buffer.from([0xff, ...Buffer.from('/planted.txt')]), join(sandbox.root, 'W'));
		await assert_fs_error_rejects(fs_open_path(join(sandbox.root, 'W')), 'io_error');
		await assert_fs_error_rejects(fs_remove_path(join(sandbox.root, 'W/x')), 'io_error');
		assert.ok(lstatSync(join(sandbox.root, 'W')).isSymbolicLink());
	});

	test('a path whose real path is not UTF-8 is refused, never a lossy path', async () => {
		mkdirSync(Buffer.concat([Buffer.from(sandbox.root + '/'), Buffer.from([0xfe])]));
		symlinkSync(Buffer.from([0xfe]), join(sandbox.root, 'R'));
		await assert_fs_error_rejects(fs_open_path(join(sandbox.root, 'R')), 'io_error');
		await assert_fs_error_rejects(fs_open_path(join(sandbox.root, 'R/x.txt')), 'io_error');
		await assert_fs_error_rejects(fs_open_dir_path(join(sandbox.root, 'R')), 'io_error');
	});
});

describe('fs_open_dir_path', () => {
	test('opens a directory outside the project as its own handle', async () => {
		mkdirSync(join(sandbox.root, 'sub'));
		const opened = await fs_open_dir_path(join(sandbox.root, 'sub'));
		assert.ok(opened);
		assert.strictEqual(opened.dir.path, join(sandbox.root, 'sub'));
		assert.strictEqual(opened.path, '');
	});

	test('follows a symlink to the directory', async () => {
		symlinkSync(sandbox.outside, join(sandbox.root, 'link'));
		const opened = await fs_open_dir_path(join(sandbox.root, 'link'));
		assert.ok(opened);
		assert.strictEqual(opened.dir.path, sandbox.outside);
		assert.strictEqual(await opened.dir.read_text('secret.txt'), 'secret');
	});

	test('a directory beneath the project is `fs.project` and its path', async () => {
		const opened = await fs_open_dir_path(join(fs.project.path, 'src/lib/'));
		assert.ok(opened);
		assert.strictEqual(opened.dir, fs.project);
		assert.strictEqual(opened.path, 'src/lib');
	});

	test('a missing directory is `null`', async () => {
		assert.strictEqual(await fs_open_dir_path(join(sandbox.root, 'missing/deeper')), null);
	});

	test('a file outside the project throws `not_a_directory`', async () => {
		writeFileSync(join(sandbox.root, 'a.txt'), 'a');
		await assert_fs_error_rejects(fs_open_dir_path(join(sandbox.root, 'a.txt')), 'not_a_directory');
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

describe('fs_copy_path', () => {
	test('copies a file between directories outside the project', async () => {
		writeFileSync(join(sandbox.root, 'a.txt'), 'a');
		await fs_copy_path(join(sandbox.root, 'a.txt'), join(sandbox.outside, 'b.txt'));
		assert.strictEqual(readFileSync(join(sandbox.outside, 'b.txt'), 'utf8'), 'a');
	});

	test('copies a tree, its symlinks as links', async () => {
		mkdirSync(join(sandbox.root, 'tree/sub'), { recursive: true });
		writeFileSync(join(sandbox.root, 'tree/sub/a.txt'), 'a');
		symlinkSync('sub/a.txt', join(sandbox.root, 'tree/link'));
		await fs_copy_path(join(sandbox.root, 'tree'), join(sandbox.outside, 'copied'), {
			recursive: true
		});
		assert.strictEqual(readFileSync(join(sandbox.outside, 'copied/sub/a.txt'), 'utf8'), 'a');
		assert.ok(lstatSync(join(sandbox.outside, 'copied/link')).isSymbolicLink());
		assert.strictEqual(readFileSync(join(sandbox.outside, 'copied/link'), 'utf8'), 'a');
	});

	test('follows symlinks above both paths', async () => {
		writeFileSync(join(sandbox.root, 'a.txt'), 'a');
		symlinkSync(sandbox.root, join(sandbox.base, 'root_link'));
		symlinkSync(sandbox.outside, join(sandbox.base, 'outside_link'));
		await fs_copy_path(
			join(sandbox.base, 'root_link/a.txt'),
			join(sandbox.base, 'outside_link/b.txt')
		);
		assert.strictEqual(readFileSync(join(sandbox.outside, 'b.txt'), 'utf8'), 'a');
	});

	test('a missing source throws `not_found`', async () => {
		await assert_fs_error_rejects(
			fs_copy_path(join(sandbox.root, 'missing.txt'), join(sandbox.outside, 'b.txt')),
			'not_found'
		);
	});
});

describe('fs_empty_dir_path', () => {
	test('empties a directory through a symlink, keeping the link and the directory', async () => {
		mkdirSync(join(sandbox.outside, 'sub'));
		writeFileSync(join(sandbox.outside, 'sub/a.txt'), 'a');
		symlinkSync(sandbox.outside, join(sandbox.root, 'link'));
		await fs_empty_dir_path(join(sandbox.root, 'link'));
		assert.ok(lstatSync(join(sandbox.root, 'link')).isSymbolicLink());
		assert.ok(lstatSync(sandbox.outside).isDirectory());
		assert.deepEqual(readdirSync(sandbox.outside), []);
	});

	test('removes a symlink inside it, never what it points at', async () => {
		mkdirSync(join(sandbox.root, 'dir'));
		symlinkSync(sandbox.outside, join(sandbox.root, 'dir/link'));
		await fs_empty_dir_path(join(sandbox.root, 'dir'));
		assert.deepEqual(readdirSync(join(sandbox.root, 'dir')), []);
		assert.strictEqual(readFileSync(join(sandbox.outside, 'secret.txt'), 'utf8'), 'secret');
	});

	test('a missing directory, or a dangling symlink, is a no-op', async () => {
		await fs_empty_dir_path(join(sandbox.root, 'missing'));
		symlinkSync('missing', join(sandbox.root, 'dangling'));
		await fs_empty_dir_path(join(sandbox.root, 'dangling'));
		assert.ok(lstatSync(join(sandbox.root, 'dangling')).isSymbolicLink());
	});

	test('refuses the project and its ancestors, through a symlink too', async () => {
		symlinkSync(dirname(process.cwd()), join(sandbox.root, 'up'));
		for (const path of ['.', '..', join(sandbox.root, 'up')]) {
			await assert_fs_error_rejects(fs_empty_dir_path(path), 'io_error');
		}
		// the refusal names the resolved target and the path as given
		const error = await assert_fs_error_rejects(
			fs_empty_dir_path(join(sandbox.root, 'up')),
			'io_error'
		);
		assert.include(error.message, `${dirname(process.cwd())} (from ${join(sandbox.root, 'up')})`);
		// the root goes through `is_protected_dir` alone, never an op on it
		assert.ok(existsSync(join(process.cwd(), 'package.json')));
		assert.ok(existsSync(join(sandbox.outside, 'secret.txt')));
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
		assert.ok(existsSync(join(sandbox.outside, 'secret.txt')));
	});
});
