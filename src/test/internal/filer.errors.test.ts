import { test, assert, vi } from 'vitest';
import { join, resolve } from 'node:path';
import { chmod, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { execFileSync } from 'node:child_process';
import { pathToFileURL } from 'node:url';

import type { WatchNodeFs, WatcherChange } from '#lib/internal/watch_dir.ts';
import { Filer, filter_dependents } from '#lib/internal/filer.ts';
import type { Disknode } from '#lib/internal/disknode.ts';

import { create_mock_logger } from '../test_helpers.ts';

const fixtures_dir = resolve(import.meta.dirname, '../fixtures');

const create_test_disknode = (id: string, contents: string | null = null): Disknode => ({
	id,
	contents,
	external: false,
	ctime: 1,
	mtime: 1,
	content_hash: null,
	dependents: new Map(),
	dependencies: new Map()
});

// Initialization error tests
test('error in watch_dir.init() is handled', async () => {
	const mock_watch_dir = vi.fn(() => {
		const mock_watcher: WatchNodeFs = {
			init: vi.fn(async () => {
				throw new Error('Initialization failed');
			}),
			close: vi.fn(async () => {})
		};
		return mock_watcher;
	});

	const filer = new Filer({ watch_dir: mock_watch_dir });

	// init should propagate the error
	let error_thrown = false;
	try {
		await filer.init();
	} catch (error) {
		error_thrown = true;
		assert.ok(error instanceof Error);
		assert.match(error.message, /Initialization failed/);
	}

	assert.ok(error_thrown, 'Should have thrown an error');

	// Filer should not be marked as initialized
	assert.equal(filer.inited, false);
});

test('watcher is cleaned up when init fails after partial success', async () => {
	let mock_watcher: WatchNodeFs;
	const mock_watch_dir = vi.fn((options) => {
		mock_watcher = {
			init: vi.fn(async () => {
				// Add some files first
				options.on_change({ type: 'add', path: '/test/file1.ts', is_directory: false });
				// Then fail
				throw new Error('Init failed after partial success');
			}),
			close: vi.fn(async () => {})
		};
		return mock_watcher;
	});

	const filer = new Filer({ watch_dir: mock_watch_dir });

	// Init should fail
	let error_thrown = false;
	try {
		await filer.init();
	} catch (error) {
		error_thrown = true;
		assert.ok(error instanceof Error);
		assert.match(error.message, /Init failed after partial success/);
	}
	assert.ok(error_thrown, 'Should have thrown an error');

	// Watcher should have been cleaned up
	assert.equal(
		(mock_watcher!.close as any).mock.calls.length,
		1,
		'watcher.close() should be called on error'
	);
	assert.equal(filer.inited, false);
	assert.equal(filer.files.size, 0);
});

test('can reinitialize after init error', async () => {
	let should_fail = true;
	const mock_watch_dir = vi.fn((options) => {
		const mock_watcher: WatchNodeFs = {
			init: vi.fn(async () => {
				if (should_fail) {
					throw new Error('First init fails');
				}
				// Second init succeeds
				options.on_change({ type: 'add', path: '/test/file1.ts', is_directory: false });
			}),
			close: vi.fn(async () => {})
		};
		return mock_watcher;
	});

	const filer = new Filer({ watch_dir: mock_watch_dir });

	// First init should fail
	let error_thrown = false;
	try {
		await filer.init();
	} catch (error) {
		error_thrown = true;
		assert.ok(error instanceof Error);
		assert.match(error.message, /First init fails/);
	}
	assert.ok(error_thrown, 'Should have thrown an error');

	assert.equal(filer.inited, false);

	// Second init should succeed
	should_fail = false;
	await filer.init();

	assert.equal(filer.inited, true);
	assert.equal(filer.files.size, 1);
});

test('handles error when watcher.close() fails during init error', async () => {
	let mock_watcher: WatchNodeFs;
	let close_error_thrown = false;

	const mock_watch_dir = vi.fn((options) => {
		mock_watcher = {
			init: vi.fn(async () => {
				options.on_change({ type: 'add', path: '/test/file1.ts', is_directory: false });
				throw new Error('Init failed');
			}),
			close: vi.fn(async () => {
				close_error_thrown = true;
				throw new Error('Close also failed');
			})
		};
		return mock_watcher;
	});

	const filer = new Filer({ watch_dir: mock_watch_dir });

	// Init should fail with the original error
	let error_thrown = false;
	try {
		await filer.init();
	} catch (error) {
		error_thrown = true;
		assert.ok(error instanceof Error);
		// Should get the init error, not the close error
		assert.match(error.message, /Init failed/);
	}

	assert.ok(error_thrown, 'Init should have thrown');
	assert.ok(close_error_thrown, 'Close should have been attempted');

	// Despite close error, state should still be cleaned up
	assert.equal(filer.inited, false);
	assert.equal(filer.files.size, 0);
});

test('can recover from multiple consecutive init errors', async () => {
	let attempt = 0;
	const mock_watch_dir = vi.fn((options) => {
		const mock_watcher: WatchNodeFs = {
			init: vi.fn(async () => {
				attempt++;
				if (attempt <= 2) {
					throw new Error(`Init failed attempt ${attempt}`);
				}
				// Third attempt succeeds
				options.on_change({ type: 'add', path: '/test/file1.ts', is_directory: false });
			}),
			close: vi.fn(async () => {})
		};
		return mock_watcher;
	});

	const filer = new Filer({ watch_dir: mock_watch_dir });

	// First attempt fails
	try {
		await filer.init();
		assert.fail('Should have thrown');
	} catch (error) {
		assert.match((error as Error).message, /Init failed attempt 1/);
	}
	assert.equal(filer.inited, false);

	// Second attempt fails
	try {
		await filer.init();
		assert.fail('Should have thrown');
	} catch (error) {
		assert.match((error as Error).message, /Init failed attempt 2/);
	}
	assert.equal(filer.inited, false);

	// Third attempt succeeds
	await filer.init();
	assert.equal(filer.inited, true);
	assert.equal(filer.files.size, 1);
});

// Import resolution edge cases
test('skips non-file URL schemes from import resolution', async () => {
	const fixture_path = resolve(fixtures_dir, 'filer_builtin_import.ts');

	const mock_watch_dir = vi.fn((options) => {
		const mock_watcher: WatchNodeFs = {
			init: vi.fn(async () => {
				options.on_change({ type: 'add', path: fixture_path, is_directory: false });
			}),
			close: vi.fn(async () => {})
		};
		return mock_watcher;
	});

	const filer = new Filer({ watch_dir: mock_watch_dir });
	await filer.init();

	const file = filer.get_by_id(fixture_path);
	assert.ok(file);
	assert.ok(file.contents); // file was actually read

	// node:crypto should NOT appear in dependencies (non-file URL scheme)
	assert.equal(file.dependencies.size, 0, 'builtin imports should not be tracked as dependencies');
});

test('resolves file: URLs from import resolution', async () => {
	const fixture_path = resolve(fixtures_dir, 'filer_local_import.ts');
	const dep_path = resolve(fixtures_dir, 'filer_builtin_import.ts');

	const mock_watch_dir = vi.fn((options) => {
		const mock_watcher: WatchNodeFs = {
			init: vi.fn(async () => {
				options.on_change({ type: 'add', path: fixture_path, is_directory: false });
			}),
			close: vi.fn(async () => {})
		};
		return mock_watcher;
	});

	const filer = new Filer({ watch_dir: mock_watch_dir });
	await filer.init();

	const file = filer.get_by_id(fixture_path);
	assert.ok(file);
	assert.ok(file.contents);

	// The local import should appear as a dependency
	assert.ok(file.dependencies.has(dep_path), `expected dependency on ${dep_path}`);
});

// File system edge cases
test('handles permission errors on file read', async () => {
	const mock_watch_dir = vi.fn((options) => {
		const mock_watcher: WatchNodeFs = {
			init: vi.fn(async () => {
				// Simulating a file that exists but can't be read
				options.on_change({ type: 'add', path: '/test/restricted.ts', is_directory: false });
			}),
			close: vi.fn(async () => {})
		};
		return mock_watcher;
	});

	const filer = new Filer({ watch_dir: mock_watch_dir });
	await filer.init();

	// File should be tracked even if we can't read its contents
	const file = filer.get_by_id('/test/restricted.ts');
	assert.ok(file);
	// Contents will be null since file read failed
	assert.equal(file.contents, null);
});

test('a special file like a FIFO is a contentless node, never read', async () => {
	const dir = await mkdtemp(join(tmpdir(), 'gro_filer_fifo_'));
	try {
		const fifo_path = join(dir, 'pipe.ts');
		execFileSync('mkfifo', [fifo_path]); // a read would block until a writer opens it
		const mock_watch_dir = vi.fn((options) => {
			const mock_watcher: WatchNodeFs = {
				init: vi.fn(async () => {
					options.on_change({ type: 'add', path: fifo_path, is_directory: false });
				}),
				close: vi.fn(async () => {})
			};
			return mock_watcher;
		});

		const filer = new Filer({ watch_dir: mock_watch_dir, watch_dir_options: { dir } });
		await filer.init();

		const fifo = filer.get_by_id(fifo_path);
		assert.ok(fifo);
		assert.equal(fifo.contents, null);
		assert.equal(fifo.content_hash, null);

		await filer.close();
	} finally {
		await rm(dir, { recursive: true, force: true });
	}
});

test('an unexpected error processing a change is logged, and later changes process', async () => {
	const dir = await mkdtemp(join(tmpdir(), 'gro_filer_queue_error_'));
	try {
		const file_path = join(dir, 'a.ts');
		await writeFile(file_path, 'export const a = 1;\n');
		let on_change_callback: ((change: any) => void) | null = null as any;
		const mock_watch_dir = vi.fn((options) => {
			on_change_callback = options.on_change;
			const mock_watcher: WatchNodeFs = {
				init: vi.fn(async () => {}),
				close: vi.fn(async () => {})
			};
			return mock_watcher;
		});
		const log = create_mock_logger();
		const filer = new Filer({ watch_dir: mock_watch_dir, watch_dir_options: { dir }, log });
		await filer.init();

		assert.ok(on_change_callback);
		// `ENAMETOOLONG` is no missing file, so the read rethrows it
		on_change_callback({ type: 'add', path: join(dir, 'x'.repeat(300)), is_directory: false });
		await new Promise((r) => setTimeout(r, 20));
		const errors = vi.mocked(log.error).mock.calls;
		assert.equal(errors.length, 1);
		assert.equal(errors[0]![0], '[filer] failed to process changes');
		assert.equal((errors[0]![1] as NodeJS.ErrnoException).code, 'ENAMETOOLONG');

		on_change_callback({ type: 'add', path: file_path, is_directory: false });
		await new Promise((r) => setTimeout(r, 20));
		assert.equal(filer.get_by_id(file_path)?.contents, 'export const a = 1;\n');

		await filer.close();
	} finally {
		await rm(dir, { recursive: true, force: true });
	}
});

test('a failing read in a burst is logged once, and the changes behind it process', async () => {
	const dir = await mkdtemp(join(tmpdir(), 'gro_filer_queue_error_burst_'));
	try {
		const a_path = join(dir, 'a.ts');
		const b_path = join(dir, 'b.ts');
		await writeFile(a_path, 'export const a = 1;\n');
		await writeFile(b_path, 'export const b = 1;\n');
		const mock_watch_dir = vi.fn((options) => {
			const mock_watcher: WatchNodeFs = {
				// the burst arrives during init, which still succeeds
				init: vi.fn(async () => {
					for (const path of [a_path, join(dir, 'x'.repeat(300)), b_path]) {
						options.on_change({ type: 'add', path, is_directory: false });
					}
				}),
				close: vi.fn(async () => {})
			};
			return mock_watcher;
		});
		const log = create_mock_logger();
		const filer = new Filer({ watch_dir: mock_watch_dir, watch_dir_options: { dir }, log });
		await filer.init();

		assert.equal(filer.get_by_id(a_path)?.contents, 'export const a = 1;\n');
		assert.equal(filer.get_by_id(b_path)?.contents, 'export const b = 1;\n');
		const errors = vi.mocked(log.error).mock.calls;
		assert.equal(errors.length, 1);
		assert.equal((errors[0]![1] as NodeJS.ErrnoException).code, 'ENAMETOOLONG');

		await filer.close();
	} finally {
		await rm(dir, { recursive: true, force: true });
	}
});

test('after init, a change queued behind a failing one processes with no further change', async () => {
	const dir = await mkdtemp(join(tmpdir(), 'gro_filer_queue_error_recheck_'));
	try {
		const a_path = join(dir, 'a.ts');
		await writeFile(a_path, 'export const a = 1;\n');
		let on_change: ((change: WatcherChange) => void) | undefined;
		const mock_watch_dir = vi.fn((options) => {
			on_change = options.on_change;
			const mock_watcher: WatchNodeFs = {
				init: vi.fn(async () => {}),
				close: vi.fn(async () => {})
			};
			return mock_watcher;
		});
		const log = create_mock_logger();
		const filer = new Filer({ watch_dir: mock_watch_dir, watch_dir_options: { dir }, log });
		await filer.init();

		// no drain runs after init, so only the recheck after the failed run takes `a.ts`
		on_change!({ type: 'add', path: join(dir, 'x'.repeat(300)), is_directory: false });
		on_change!({ type: 'add', path: a_path, is_directory: false });
		await new Promise((resolve) => setTimeout(resolve, 20));

		assert.equal(filer.get_by_id(a_path)?.contents, 'export const a = 1;\n');
		assert.equal(vi.mocked(log.error).mock.calls.length, 1);

		await filer.close();
	} finally {
		await rm(dir, { recursive: true, force: true });
	}
});

// root reads a file with no permissions
test.skipIf(process.getuid?.() === 0)(
	'a file that fails to read after its stat has no contents and no times',
	async () => {
		const dir = await mkdtemp(join(tmpdir(), 'gro_filer_unreadable_'));
		const file_path = join(dir, 'a.ts');
		try {
			await writeFile(file_path, 'export const a = 1;\n');
			await chmod(file_path, 0o000); // the stat succeeds and the read fails with `EACCES`
			const mock_watch_dir = vi.fn((options) => {
				const mock_watcher: WatchNodeFs = {
					init: vi.fn(async () => {
						options.on_change({ type: 'add', path: file_path, is_directory: false });
					}),
					close: vi.fn(async () => {})
				};
				return mock_watcher;
			});
			const filer = new Filer({ watch_dir: mock_watch_dir, watch_dir_options: { dir } });
			await filer.init();

			const file = filer.get_by_id(file_path);
			assert.ok(file);
			assert.equal(file.contents, null);
			assert.equal(file.ctime, null);
			assert.equal(file.mtime, null);

			await filer.close();
		} finally {
			await chmod(file_path, 0o644).catch(() => {}); // absent when the write failed
			await rm(dir, { recursive: true, force: true });
		}
	}
);

test('an unexpected error processing a change goes to `console` with no log', async () => {
	const dir = await mkdtemp(join(tmpdir(), 'gro_filer_queue_error_console_'));
	const console_error = vi.spyOn(console, 'error').mockImplementation(() => {});
	try {
		let on_change_callback: ((change: any) => void) | null = null as any;
		const mock_watch_dir = vi.fn((options) => {
			on_change_callback = options.on_change;
			const mock_watcher: WatchNodeFs = {
				init: vi.fn(async () => {}),
				close: vi.fn(async () => {})
			};
			return mock_watcher;
		});
		const filer = new Filer({ watch_dir: mock_watch_dir, watch_dir_options: { dir } });
		await filer.init();
		assert.ok(on_change_callback);
		on_change_callback({ type: 'add', path: join(dir, 'x'.repeat(300)), is_directory: false });
		await new Promise((r) => setTimeout(r, 20));
		assert.equal(console_error.mock.calls.length, 1);
		assert.equal(console_error.mock.calls[0]![0], '[filer] failed to process changes');
		await filer.close();
	} finally {
		console_error.mockRestore();
		await rm(dir, { recursive: true, force: true });
	}
});

test('an unresolvable bare import logs as an error', async () => {
	const dir = await mkdtemp(join(tmpdir(), 'gro_filer_unresolvable_'));
	try {
		const src_dir = join(dir, 'src');
		await mkdir(src_dir, { recursive: true });
		const project_path = join(src_dir, 'a.ts');
		await writeFile(project_path, "import 'gro_test_missing_package';\n");
		const mock_watch_dir = vi.fn((options) => {
			const mock_watcher: WatchNodeFs = {
				init: vi.fn(async () => {
					options.on_change({ type: 'add', path: project_path, is_directory: false });
				}),
				close: vi.fn(async () => {})
			};
			return mock_watcher;
		});
		const log = create_mock_logger();
		const filer = new Filer({
			watch_dir: mock_watch_dir,
			watch_dir_options: { dir: src_dir },
			log
		});
		await filer.init();

		const failures = vi
			.mocked(log.error)
			.mock.calls.filter((c) => c[0] === '[filer] failed to resolve path')
			.map((c) => c[2]);
		assert.deepEqual(failures, [pathToFileURL(project_path).href]);

		await filer.close();
	} finally {
		await rm(dir, { recursive: true, force: true });
	}
});

test('an external dependency naming a directory is a contentless node, not an init failure', async () => {
	const dir = await mkdtemp(join(tmpdir(), 'gro_filer_dir_dep_'));
	try {
		const root_dir = join(dir, 'src', 'lib');
		await mkdir(root_dir, { recursive: true });
		const importer_path = join(root_dir, 'a.ts');
		await writeFile(importer_path, "import '../../';\nexport const a = 1;\n");
		const dir_dep_path = dir + '/';

		const mock_watch_dir = vi.fn((options) => {
			const mock_watcher: WatchNodeFs = {
				init: vi.fn(async () => {
					options.on_change({ type: 'add', path: importer_path, is_directory: false });
				}),
				close: vi.fn(async () => {})
			};
			return mock_watcher;
		});

		const filer = new Filer({ watch_dir: mock_watch_dir, watch_dir_options: { dir: root_dir } });
		await filer.init(); // drains the deferred external add, rejected with `EISDIR` when rethrown

		const importer = filer.get_by_id(importer_path);
		assert.ok(importer);
		assert.ok(importer.dependencies.has(dir_dep_path), 'importer should depend on the directory');
		const dir_dep = filer.get_by_id(dir_dep_path);
		assert.ok(dir_dep);
		assert.equal(dir_dep.external, true);
		assert.equal(dir_dep.contents, null);
		assert.equal(dir_dep.content_hash, null);

		await filer.close();
	} finally {
		await rm(dir, { recursive: true, force: true });
	}
});
test('an external dependency through a file is a contentless node, not an init failure', async () => {
	const dir = await mkdtemp(join(tmpdir(), 'gro_filer_notdir_dep_'));
	try {
		const root_dir = join(dir, 'src', 'lib');
		await mkdir(root_dir, { recursive: true });
		await writeFile(join(dir, 'src', 'x.ts'), 'export const x = 1;\n');
		const importer_path = join(root_dir, 'a.ts');
		await writeFile(importer_path, "import '../x.ts/y';\nexport const a = 1;\n");
		const notdir_dep_path = join(dir, 'src', 'x.ts', 'y');

		const mock_watch_dir = vi.fn((options) => {
			const mock_watcher: WatchNodeFs = {
				init: vi.fn(async () => {
					options.on_change({ type: 'add', path: importer_path, is_directory: false });
				}),
				close: vi.fn(async () => {})
			};
			return mock_watcher;
		});

		const filer = new Filer({ watch_dir: mock_watch_dir, watch_dir_options: { dir: root_dir } });
		await filer.init(); // drains the deferred external add, rejected with `ENOTDIR` when rethrown

		const importer = filer.get_by_id(importer_path);
		assert.ok(importer);
		assert.ok(
			importer.dependencies.has(notdir_dep_path),
			'importer should depend on the path through the file'
		);
		const notdir_dep = filer.get_by_id(notdir_dep_path);
		assert.ok(notdir_dep);
		assert.equal(notdir_dep.external, true);
		assert.equal(notdir_dep.contents, null);
		assert.equal(notdir_dep.content_hash, null);

		await filer.close();
	} finally {
		await rm(dir, { recursive: true, force: true });
	}
});

test('handles same mtime but different contents', async () => {
	let on_change_callback: ((change: any) => void) | null = null as any;

	const mock_watch_dir = vi.fn((options) => {
		on_change_callback = options.on_change;
		const mock_watcher: WatchNodeFs = {
			init: vi.fn(async () => {
				options.on_change({ type: 'add', path: '/test/file.ts', is_directory: false });
			}),
			close: vi.fn(async () => {})
		};
		return mock_watcher;
	});

	const filer = new Filer({ watch_dir: mock_watch_dir });
	await filer.init();

	const file_before = filer.get_by_id('/test/file.ts');
	assert.ok(file_before);
	const mtime_before = file_before.mtime;

	// Simulate update with same mtime
	assert.ok(on_change_callback);
	on_change_callback({ type: 'update', path: '/test/file.ts', is_directory: false });

	// Wait for queue processing
	await new Promise((resolve) => setTimeout(resolve, 10));

	const file_after = filer.get_by_id('/test/file.ts');
	assert.ok(file_after);
	// mtime should be tracked
	assert.equal(file_after.mtime, mtime_before);
});

test('handles parse_imports throwing error', async () => {
	const mock_watch_dir = vi.fn((options) => {
		const mock_watcher: WatchNodeFs = {
			init: vi.fn(async () => {
				// File with potentially invalid syntax
				options.on_change({ type: 'add', path: '/test/invalid.ts', is_directory: false });
			}),
			close: vi.fn(async () => {})
		};
		return mock_watcher;
	});

	const filer = new Filer({ watch_dir: mock_watch_dir });
	await filer.init();

	// File should still be tracked even if parse_imports fails
	const file = filer.get_by_id('/test/invalid.ts');
	assert.ok(file);
	assert.equal(file.id, '/test/invalid.ts');
	// Dependencies should be empty when parsing fails
	assert.equal(file.dependencies.size, 0);
});

// Performance edge cases
test('handles very deep dependency chain', async () => {
	// Create a chain of 50 files: a <- b <- c <- ... <- z...
	const depth = 50;
	const files: Array<Disknode> = [];

	for (let i = 0; i < depth; i++) {
		const file = create_test_disknode(
			`/test/file${i}.ts`,
			i > 0 ? `import {x} from './file${i - 1}'` : 'export const x = 1'
		);
		files.push(file);

		// Link dependencies
		if (i > 0) {
			const prev_file = files[i - 1]!;
			file.dependencies.set(prev_file.id, prev_file);
			prev_file.dependents.set(file.id, file);
		}
	}

	const get_by_id = (id: string) => files.find((f) => f.id === id);

	// Should complete without stack overflow (now using iterative implementation)
	const dependents = filter_dependents(files[0]!, get_by_id);

	// All other files should be dependents of the first file
	assert.equal(dependents.size, depth - 1);
});

test('handles large file contents efficiently', async () => {
	const mock_watch_dir = vi.fn((options) => {
		const mock_watcher: WatchNodeFs = {
			init: vi.fn(async () => {
				options.on_change({ type: 'add', path: '/test/large.ts', is_directory: false });
			}),
			close: vi.fn(async () => {})
		};
		return mock_watcher;
	});

	const filer = new Filer({ watch_dir: mock_watch_dir });
	await filer.init();

	const file = filer.get_by_id('/test/large.ts');
	assert.ok(file);
	// File should be tracked regardless of content size
	assert.equal(file.id, '/test/large.ts');
});
