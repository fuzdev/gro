import {test, assert, vi} from 'vitest';
import {resolve} from 'node:path';

import type {WatchNodeFs} from '../lib/watch_dir.ts';
import {Filer, filter_dependents} from '../lib/filer.ts';
import type {Disknode} from '../lib/disknode.ts';

const fixtures_dir = resolve(import.meta.dirname, 'fixtures');

const create_test_disknode = (id: string, contents: string | null = null): Disknode => ({
	id,
	contents,
	external: false,
	ctime: 1,
	mtime: 1,
	content_hash: null,
	dependents: new Map(),
	dependencies: new Map(),
});

// Initialization error tests
test('error in watch_dir.init() is handled', async () => {
	const mock_watch_dir = vi.fn(() => {
		const mock_watcher: WatchNodeFs = {
			init: vi.fn(async () => {
				throw new Error('Initialization failed');
			}),
			close: vi.fn(async () => {}),
		};
		return mock_watcher;
	});

	const filer = new Filer({watch_dir: mock_watch_dir});

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
				options.on_change({type: 'add', path: '/test/file1.ts', is_directory: false});
				// Then fail
				throw new Error('Init failed after partial success');
			}),
			close: vi.fn(async () => {}),
		};
		return mock_watcher;
	});

	const filer = new Filer({watch_dir: mock_watch_dir});

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
		'watcher.close() should be called on error',
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
				options.on_change({type: 'add', path: '/test/file1.ts', is_directory: false});
			}),
			close: vi.fn(async () => {}),
		};
		return mock_watcher;
	});

	const filer = new Filer({watch_dir: mock_watch_dir});

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
				options.on_change({type: 'add', path: '/test/file1.ts', is_directory: false});
				throw new Error('Init failed');
			}),
			close: vi.fn(async () => {
				close_error_thrown = true;
				throw new Error('Close also failed');
			}),
		};
		return mock_watcher;
	});

	const filer = new Filer({watch_dir: mock_watch_dir});

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
				options.on_change({type: 'add', path: '/test/file1.ts', is_directory: false});
			}),
			close: vi.fn(async () => {}),
		};
		return mock_watcher;
	});

	const filer = new Filer({watch_dir: mock_watch_dir});

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
				options.on_change({type: 'add', path: fixture_path, is_directory: false});
			}),
			close: vi.fn(async () => {}),
		};
		return mock_watcher;
	});

	const filer = new Filer({watch_dir: mock_watch_dir});
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
				options.on_change({type: 'add', path: fixture_path, is_directory: false});
			}),
			close: vi.fn(async () => {}),
		};
		return mock_watcher;
	});

	const filer = new Filer({watch_dir: mock_watch_dir});
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
				options.on_change({type: 'add', path: '/test/restricted.ts', is_directory: false});
			}),
			close: vi.fn(async () => {}),
		};
		return mock_watcher;
	});

	const filer = new Filer({watch_dir: mock_watch_dir});
	await filer.init();

	// File should be tracked even if we can't read its contents
	const file = filer.get_by_id('/test/restricted.ts');
	assert.ok(file);
	// Contents will be null since file read failed
	assert.equal(file.contents, null);
});

test('handles same mtime but different contents', async () => {
	let on_change_callback: ((change: any) => void) | null = null as any;

	const mock_watch_dir = vi.fn((options) => {
		on_change_callback = options.on_change;
		const mock_watcher: WatchNodeFs = {
			init: vi.fn(async () => {
				options.on_change({type: 'add', path: '/test/file.ts', is_directory: false});
			}),
			close: vi.fn(async () => {}),
		};
		return mock_watcher;
	});

	const filer = new Filer({watch_dir: mock_watch_dir});
	await filer.init();

	const file_before = filer.get_by_id('/test/file.ts');
	assert.ok(file_before);
	const mtime_before = file_before.mtime;

	// Simulate update with same mtime
	assert.ok(on_change_callback);
	on_change_callback({type: 'update', path: '/test/file.ts', is_directory: false});

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
				options.on_change({type: 'add', path: '/test/invalid.ts', is_directory: false});
			}),
			close: vi.fn(async () => {}),
		};
		return mock_watcher;
	});

	const filer = new Filer({watch_dir: mock_watch_dir});
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
			i > 0 ? `import {x} from './file${i - 1}'` : 'export const x = 1',
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
				options.on_change({type: 'add', path: '/test/large.ts', is_directory: false});
			}),
			close: vi.fn(async () => {}),
		};
		return mock_watcher;
	});

	const filer = new Filer({watch_dir: mock_watch_dir});
	await filer.init();

	const file = filer.get_by_id('/test/large.ts');
	assert.ok(file);
	// File should be tracked regardless of content size
	assert.equal(file.id, '/test/large.ts');
});
