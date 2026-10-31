import { test, assert, onTestFinished } from 'vitest';
import { chmod, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { watch_dir } from '#lib/internal/watch_dir.ts';

// root reads a directory with no permissions
test.skipIf(process.getuid?.() === 0)(
	'a chokidar error goes to `on_error`, not thrown',
	async () => {
		const dir = await mkdtemp(join(tmpdir(), 'gro_watch_dir_error_'));
		const locked = join(dir, 'locked');
		onTestFinished(async () => {
			await chmod(locked, 0o755).catch(() => {}); // absent when `mkdir` failed
			await rm(dir, { recursive: true, force: true });
		});
		await mkdir(locked);
		await writeFile(join(locked, 'x.ts'), '');
		await chmod(locked, 0o000);
		const errors: Array<unknown> = [];
		const watcher = watch_dir({
			dir,
			on_change: () => {},
			on_error: (error) => errors.push(error)
		});
		await watcher.init();
		await watcher.close();
		assert.equal(errors.length, 1);
		assert.equal((errors[0] as NodeJS.ErrnoException).code, 'EACCES');
	}
);
