import {test, assert, vi} from 'vitest';

import type {WatchNodeFs} from '../lib/watch_dir.ts';
import {Filer} from '../lib/filer.ts';

// Create a simple mock watch_dir that simulates file discovery
const create_mock_watch_dir = () => {
	return vi.fn((options) => {
		const mock_watcher: WatchNodeFs = {
			init: vi.fn(async () => {
				// Simulate discovering files via on_change callbacks
				options.on_change({type: 'add', path: '/test/file1.ts', is_directory: false});
				options.on_change({type: 'add', path: '/test/file2.ts', is_directory: false});
				options.on_change({type: 'add', path: '/test/lib/helper.ts', is_directory: false});
			}),
			close: vi.fn(async () => {}),
		};
		return mock_watcher;
	});
};

// Close/init race condition tests
test('close() during init() handles gracefully', async () => {
	const mock_watch_dir = vi.fn((options) => {
		const mock_watcher: WatchNodeFs = {
			init: vi.fn(async () => {
				// Simulate slow initialization
				await new Promise((resolve) => setTimeout(resolve, 50));
				options.on_change({type: 'add', path: '/test/file1.ts', is_directory: false});
			}),
			close: vi.fn(async () => {}),
		};
		return mock_watcher;
	});

	const filer = new Filer({watch_dir: mock_watch_dir});

	// Start initialization but don't await
	const init_promise = filer.init();

	// Close while initializing
	await filer.close();

	// Initialization should complete without errors
	await init_promise;

	// Filer should be closed
	assert.equal(filer.inited, false);
	assert.equal(filer.files.size, 0);
});

test('concurrent init and close handle correctly', async () => {
	const mock_watch_dir = vi.fn((options) => {
		const mock_watcher: WatchNodeFs = {
			init: vi.fn(async () => {
				// Simulate slower initialization
				await new Promise((resolve) => setTimeout(resolve, 10));
				options.on_change({type: 'add', path: '/test/file1.ts', is_directory: false});
				await new Promise((resolve) => setTimeout(resolve, 10));
				options.on_change({type: 'add', path: '/test/file2.ts', is_directory: false});
			}),
			close: vi.fn(async () => {}),
		};
		return mock_watcher;
	});

	const filer = new Filer({watch_dir: mock_watch_dir});

	// Start multiple concurrent operations
	const operations = Promise.all([
		filer.init(),
		filer.init(),
		new Promise((resolve) => setTimeout(resolve, 5)).then(() => filer.close()),
		filer.init(),
	]);

	// Should complete without errors
	await operations;

	// Final state should be closed
	assert.equal(filer.inited, false);
	assert.equal(filer.files.size, 0);
});

test('closing flag is reset even if close throws', async () => {
	const mock_watch_dir = vi.fn((options) => {
		const mock_watcher: WatchNodeFs = {
			init: vi.fn(async () => {
				options.on_change({type: 'add', path: '/test/file1.ts', is_directory: false});
			}),
			close: vi.fn(async () => {
				throw new Error('Close failed');
			}),
		};
		return mock_watcher;
	});

	const filer = new Filer({watch_dir: mock_watch_dir});
	await filer.init();

	// Close should throw
	let error_thrown = false;
	try {
		await filer.close();
	} catch (error) {
		error_thrown = true;
		assert.ok(error instanceof Error);
		assert.match(error.message, /Close failed/);
	}
	assert.ok(error_thrown, 'Close should have thrown an error');

	// Should be able to init again (closing flag should be reset)
	// This will fail if #closing is still true
	await filer.init();
	assert.equal(filer.inited, true);
});

test('no partial file state after close during init', async () => {
	const mock_watch_dir = vi.fn((options) => {
		const mock_watcher: WatchNodeFs = {
			init: vi.fn(async () => {
				options.on_change({type: 'add', path: '/test/file1.ts', is_directory: false});
				await new Promise((resolve) => setTimeout(resolve, 20));
				options.on_change({type: 'add', path: '/test/file2.ts', is_directory: false});
				await new Promise((resolve) => setTimeout(resolve, 20));
				options.on_change({type: 'add', path: '/test/file3.ts', is_directory: false});
			}),
			close: vi.fn(async () => {}),
		};
		return mock_watcher;
	});

	const filer = new Filer({watch_dir: mock_watch_dir});

	// Start init and close after first file
	const init_promise = filer.init();
	await new Promise((resolve) => setTimeout(resolve, 10));
	await filer.close();

	// Wait for init to complete
	await init_promise;

	// Should have no files
	assert.equal(filer.files.size, 0, 'Should have no partial file state');
	assert.equal(filer.inited, false);
});

test('watcher.close() is called on close during init', async () => {
	let mock_watcher: WatchNodeFs;
	const mock_watch_dir = vi.fn((options) => {
		mock_watcher = {
			init: vi.fn(async () => {
				await new Promise((resolve) => setTimeout(resolve, 50));
				options.on_change({type: 'add', path: '/test/file1.ts', is_directory: false});
			}),
			close: vi.fn(async () => {}),
		};
		return mock_watcher;
	});

	const filer = new Filer({watch_dir: mock_watch_dir});

	// Start init but don't await
	const init_promise = filer.init();

	// Close while initializing
	await filer.close();
	await init_promise;

	// Watcher should have been closed
	assert.equal(
		(mock_watcher!.close as any).mock.calls.length,
		1,
		'watcher.close() should be called',
	);
});

test('concurrent close() calls share same promise', async () => {
	let close_call_count = 0;
	let close_resolve: () => void;
	const close_promise: Promise<void> = new Promise((resolve) => {
		close_resolve = resolve;
	});

	const mock_watcher = {
		init: vi.fn(),
		close: vi.fn(async () => {
			close_call_count++;
			// Wait for our signal to complete
			await close_promise;
		}),
	};

	const mock_watch_dir = vi.fn(() => mock_watcher);
	const filer = new Filer({watch_dir: mock_watch_dir});

	// Initialize first
	await filer.init();
	assert.equal(filer.inited, true);

	// Call close multiple times concurrently while close is still pending
	const close1 = filer.close();
	const close2 = filer.close();
	const close3 = filer.close();

	// All should be the same promise instance since close hasn't completed yet
	assert.equal(close1, close2, 'close1 and close2 should be the same promise');
	assert.equal(close2, close3, 'close2 and close3 should be the same promise');

	// Now let the close complete
	close_resolve!();

	// Wait for all to complete
	await Promise.all([close1, close2, close3]);

	// close() should only be called once on the watcher
	assert.equal(close_call_count, 1, 'watcher.close() should only be called once');
	assert.equal(filer.inited, false);

	// After close completes, subsequent calls should return immediately
	const close4 = filer.close();
	const close5 = filer.close();
	await Promise.all([close4, close5]);

	// Still only one close call
	assert.equal(close_call_count, 1, 'watcher.close() should still only be called once');
});

// Async queue processing tests
test('processes rapid changes to different files in order', async () => {
	const events: Array<{type: string; path: string}> = [];

	const mock_watch_dir = vi.fn((options) => {
		const mock_watcher: WatchNodeFs = {
			init: vi.fn(async () => {
				// Simulate rapid-fire changes to different files
				options.on_change({type: 'add', path: '/test/file1.ts', is_directory: false});
				options.on_change({type: 'add', path: '/test/file2.ts', is_directory: false});
				options.on_change({type: 'add', path: '/test/file3.ts', is_directory: false});
				options.on_change({type: 'add', path: '/test/file4.ts', is_directory: false});
			}),
			close: vi.fn(async () => {}),
		};
		return mock_watcher;
	});

	const filer = new Filer({watch_dir: mock_watch_dir});

	await filer.watch((change) => {
		events.push({type: change.type, path: change.path});
	});

	// All events should be processed in the order they arrived
	assert.equal(events.length, 4);
	assert.equal(events[0]!.path, '/test/file1.ts');
	assert.equal(events[1]!.path, '/test/file2.ts');
	assert.equal(events[2]!.path, '/test/file3.ts');
	assert.equal(events[3]!.path, '/test/file4.ts');
});

test('queue is fully drained after init completes', async () => {
	const events: Array<string> = [];
	const mock_watch_dir = create_mock_watch_dir();
	const filer = new Filer({watch_dir: mock_watch_dir});

	await filer.init();

	// Add listener after init - should sync all existing files
	await filer.watch((change) => {
		events.push(change.path);
	});

	// All 3 files from mock should have been synced, meaning queue was drained
	assert.equal(events.length, 3);
	assert.equal(filer.files.size, 3);
});

test('new changes are queued while processing', async () => {
	let init_resolve: () => void;
	const init_complete: Promise<void> = new Promise((resolve) => {
		init_resolve = resolve;
	});

	const events: Array<string> = [];

	const mock_watch_dir = vi.fn((options) => {
		const mock_watcher: WatchNodeFs = {
			init: vi.fn(async () => {
				// Add first file
				options.on_change({type: 'add', path: '/test/file1.ts', is_directory: false});

				// Wait briefly then add another file (simulating change during processing)
				await new Promise((resolve) => setTimeout(resolve, 5));
				options.on_change({type: 'add', path: '/test/file2.ts', is_directory: false});

				init_resolve();
			}),
			close: vi.fn(async () => {}),
		};
		return mock_watcher;
	});

	const filer = new Filer({watch_dir: mock_watch_dir});

	await filer.watch((change) => {
		events.push(change.path);
	});

	await init_complete;

	// Both files should be processed
	assert.equal(events.length, 2);
	assert.ok(events.includes('/test/file1.ts'));
	assert.ok(events.includes('/test/file2.ts'));

	// Both files should exist in filer state
	assert.equal(filer.files.size, 2);
});

test('handles many rapid changes efficiently', async () => {
	const file_count = 100;
	const events: Array<{type: string; path: string}> = [];

	const mock_watch_dir = vi.fn((options) => {
		const mock_watcher: WatchNodeFs = {
			init: vi.fn(async () => {
				// Simulate many rapid file additions
				for (let i = 0; i < file_count; i++) {
					options.on_change({
						type: 'add',
						path: `/test/file${i}.ts`,
						is_directory: false,
					});
				}
			}),
			close: vi.fn(async () => {}),
		};
		return mock_watcher;
	});

	const filer = new Filer({watch_dir: mock_watch_dir});

	await filer.watch((change) => {
		events.push({type: change.type, path: change.path});
	});

	// All files should be processed
	assert.equal(events.length, file_count);

	// All files should exist in filer
	assert.equal(filer.files.size, file_count);
});

test('queue processes all changes even when they arrive during processing', async () => {
	let first_batch_complete: () => void;
	const first_batch_promise: Promise<void> = new Promise((resolve) => {
		first_batch_complete = resolve;
	});

	const events: Array<string> = [];

	const mock_watch_dir = vi.fn((options) => {
		const mock_watcher: WatchNodeFs = {
			init: vi.fn(async () => {
				// Add initial files
				options.on_change({type: 'add', path: '/test/file1.ts', is_directory: false});

				// Wait a bit then add more while processing
				await new Promise((resolve) => setTimeout(resolve, 10));
				options.on_change({type: 'add', path: '/test/file2.ts', is_directory: false});
				options.on_change({type: 'add', path: '/test/file3.ts', is_directory: false});

				first_batch_complete();
			}),
			close: vi.fn(async () => {}),
		};
		return mock_watcher;
	});

	const filer = new Filer({watch_dir: mock_watch_dir});

	await filer.watch((change) => {
		events.push(change.path);
	});

	await first_batch_promise;

	// All files should be processed, even those added during async processing
	assert.equal(events.length, 3);
	assert.ok(events.includes('/test/file1.ts'));
	assert.ok(events.includes('/test/file2.ts'));
	assert.ok(events.includes('/test/file3.ts'));
});

test('files with no actual changes do not re-notify', async () => {
	let update_count = 0;

	const mock_watch_dir = vi.fn((options) => {
		const mock_watcher: WatchNodeFs = {
			init: vi.fn(async () => {
				// First add creates the disknode (triggers notification even though file doesn't exist)
				options.on_change({type: 'add', path: '/test/file.ts', is_directory: false});
				// Second update sees no change (content_hash both null → null, no notification)
				options.on_change({type: 'update', path: '/test/file.ts', is_directory: false});
				// Third update also sees no change
				options.on_change({type: 'update', path: '/test/file.ts', is_directory: false});
			}),
			close: vi.fn(async () => {}),
		};
		return mock_watcher;
	});

	const filer = new Filer({watch_dir: mock_watch_dir});

	await filer.watch(() => {
		update_count++;
	});

	// Only the first add should notify (creates disknode), subsequent updates see no change (hash-based detection)
	assert.equal(update_count, 1);
});

test('handles rapid updates to same file', async () => {
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

	// Simulate 100 rapid updates to the same file
	assert.ok(on_change_callback);
	for (let i = 0; i < 100; i++) {
		on_change_callback({type: 'update', path: '/test/file.ts', is_directory: false});
	}

	// Wait for all processing to complete
	await new Promise((resolve) => setTimeout(resolve, 50));

	// File should still be in consistent state
	const file = filer.get_by_id('/test/file.ts');
	assert.ok(file);
	assert.equal(file.id, '/test/file.ts');
});
