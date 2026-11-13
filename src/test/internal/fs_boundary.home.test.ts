import { describe, test, assert, vi, afterEach } from 'vitest';
import { mkdirSync, realpathSync, symlinkSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

/*

`$HOME`'s real path joins the protected dirs, resolved through the `Fs` passed in.
`HOME` is read when the boundary module loads, so each test sets it and reloads.

*/

const saved_home = globalThis.process.env.HOME;
let base: string | undefined;

afterEach(() => {
	if (saved_home === undefined) delete globalThis.process.env.HOME;
	else globalThis.process.env.HOME = saved_home;
	vi.resetModules();
	if (base) rmSync(base, { recursive: true, force: true });
	base = undefined;
});

describe('to_protected_dirs and `$HOME`', () => {
	test('a symlinked `$HOME` protects its real path too, read through the `Fs` given', async () => {
		base = realpathSync(mkdtempSync(join(tmpdir(), 'gro-home-')));
		mkdirSync(join(base, 'real_home'));
		symlinkSync(join(base, 'real_home'), join(base, 'home'));
		globalThis.process.env.HOME = join(base, 'home');
		vi.resetModules();
		const { fs } = await import('#lib/fs.ts');
		const { to_protected_dirs, is_protected_dir } = await import('#lib/internal/fs_boundary.ts');
		const resolved: Array<string> = [];
		const dirs = await to_protected_dirs(
			{
				...fs,
				real_path: async (path) => {
					resolved.push(path);
					return fs.real_path(path);
				}
			},
			join(base, 'project')
		);
		assert.include(dirs, join(base, 'home'));
		assert.include(dirs, join(base, 'real_home'));
		assert.ok(is_protected_dir(join(base, 'real_home'), dirs));
		assert.deepEqual(resolved, [join(base, 'home')]);
	});
});
