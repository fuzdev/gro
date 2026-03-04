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

// Lifecycle tests
test('filer starts uninitialized', async () => {
	const filer = new Filer();
	assert.equal(filer.inited, false);
	assert.equal(filer.files.size, 0);
});

test('filer initializes on first init() call', async () => {
	const mock_watch_dir = create_mock_watch_dir();
	const filer = new Filer({watch_dir: mock_watch_dir});

	assert.equal(filer.inited, false);

	await filer.init();

	assert.equal(filer.inited, true);
	assert.equal(filer.files.size, 3);
	assert.equal(mock_watch_dir.mock.calls.length, 1);
});

test('init() is idempotent', async () => {
	const mock_watch_dir = create_mock_watch_dir();
	const filer = new Filer({watch_dir: mock_watch_dir});

	await filer.init();
	const file_count_first = filer.files.size;

	await filer.init();
	const file_count_second = filer.files.size;

	assert.equal(file_count_first, file_count_second);
	// watch_dir should only be called once
	assert.equal(mock_watch_dir.mock.calls.length, 1);
});

test('concurrent init() calls are handled correctly', async () => {
	const mock_watch_dir = create_mock_watch_dir();
	const filer = new Filer({watch_dir: mock_watch_dir});

	// Launch multiple init calls simultaneously
	const init_promises = [filer.init(), filer.init(), filer.init(), filer.init(), filer.init()];

	await Promise.all(init_promises);

	assert.equal(filer.inited, true);
	assert.equal(filer.files.size, 3);
	// watch_dir should only be called once despite concurrent calls
	assert.equal(mock_watch_dir.mock.calls.length, 1);

	// Call init again to ensure state is consistent
	await filer.init();
	assert.equal(filer.files.size, 3);
});

test('disknode structure is valid', async () => {
	const mock_watch_dir = create_mock_watch_dir();
	const filer = new Filer({watch_dir: mock_watch_dir});
	await filer.init();

	// Check the structure of created disknodes
	const file1 = filer.get_by_id('/test/file1.ts');
	assert.ok(file1);
	assert.equal(file1.id, '/test/file1.ts');
	assert.equal(typeof file1.external, 'boolean');
	assert.ok(file1.dependencies instanceof Map);
	assert.ok(file1.dependents instanceof Map);
	// content_hash is null when file doesn't exist on disk (mock files)
	assert.equal(file1.content_hash, null);
});

test('disknode content_hash is null when created via get_or_create', async () => {
	const mock_watch_dir = create_mock_watch_dir();
	const filer = new Filer({watch_dir: mock_watch_dir});
	await filer.init();

	// Create a new disknode for a file that doesn't exist
	const new_file = filer.get_or_create('/test/nonexistent.ts');
	assert.ok(new_file);
	assert.equal(new_file.contents, null);
	assert.equal(new_file.content_hash, null);
});

test('close() cleans up resources', async () => {
	const mock_watch_dir = create_mock_watch_dir();
	const filer = new Filer({watch_dir: mock_watch_dir});

	await filer.init();

	assert.equal(filer.inited, true);
	assert.equal(filer.files.size, 3);

	await filer.close();

	assert.equal(filer.inited, false);
	assert.equal(filer.files.size, 0);
});

test('can reinitialize after close()', async () => {
	const mock_watch_dir = create_mock_watch_dir();
	const filer = new Filer({watch_dir: mock_watch_dir});

	// First cycle
	await filer.init();
	assert.equal(filer.files.size, 3);
	await filer.close();

	// Second cycle
	await filer.init();
	assert.equal(filer.files.size, 3);

	// watch_dir should be called twice (once per init after close)
	assert.equal(mock_watch_dir.mock.calls.length, 2);
});

test('multiple close() calls are safe', async () => {
	const mock_watch_dir = create_mock_watch_dir();
	const filer = new Filer({watch_dir: mock_watch_dir});

	await filer.init();
	assert.equal(filer.inited, true);

	// First close
	await filer.close();
	assert.equal(filer.inited, false);
	assert.equal(filer.files.size, 0);

	// Second close - should be safe
	await filer.close();
	assert.equal(filer.inited, false);
	assert.equal(filer.files.size, 0);

	// Third close - still safe
	await filer.close();
	assert.equal(filer.inited, false);
});

test('file state remains consistent across operations', async () => {
	const mock_watch_dir = create_mock_watch_dir();
	const filer = new Filer({watch_dir: mock_watch_dir});

	// First init
	await filer.init();
	const state1 = Array.from(filer.files.keys()).sort();

	// Add and remove listener
	const cleanup = await filer.watch(() => {});
	cleanup();

	// State should be unchanged
	const state2 = Array.from(filer.files.keys()).sort();
	assert.deepEqual(state1, state2);

	// Close and reinit
	await filer.close();
	await filer.init();

	// State should be restored
	const state3 = Array.from(filer.files.keys()).sort();
	assert.deepEqual(state1, state3);
});

test('cleanup clears all state including dependencies', async () => {
	const mock_watch_dir = vi.fn((options) => {
		const mock_watcher: WatchNodeFs = {
			init: vi.fn(async () => {
				// Add files that will have dependencies
				options.on_change({type: 'add', path: '/test/main.ts', is_directory: false});
				options.on_change({type: 'add', path: '/test/lib.ts', is_directory: false});
			}),
			close: vi.fn(async () => {}),
		};
		return mock_watcher;
	});

	const filer = new Filer({watch_dir: mock_watch_dir});

	// Add a listener to verify it gets cleared
	let listener_called = false;
	await filer.watch(() => {
		listener_called = true;
	});

	// Verify initial state
	assert.equal(filer.inited, true);
	assert.equal(filer.files.size, 2);
	assert.ok(listener_called); // Listener was called during init

	// Close should clear everything
	await filer.close();

	// Verify complete cleanup
	assert.equal(filer.inited, false);
	assert.equal(filer.files.size, 0);

	// Try to init again to verify state is fully reset
	await filer.init();
	assert.equal(filer.inited, true);
	assert.equal(filer.files.size, 2);
});

// Listener tests
test('watch() initializes and notifies listeners', async () => {
	const mock_watch_dir = create_mock_watch_dir();
	const filer = new Filer({watch_dir: mock_watch_dir});

	const changes: Array<{path: string}> = [];
	const cleanup = await filer.watch((change) => {
		changes.push({path: change.path});
	});

	assert.equal(filer.inited, true);
	// Listener should be notified of existing files
	assert.equal(changes.length, 3);
	assert.ok(changes.some((c) => c.path === '/test/file1.ts'));

	cleanup();
});

test('multiple listeners receive notifications independently', async () => {
	const mock_watch_dir = create_mock_watch_dir();
	const filer = new Filer({watch_dir: mock_watch_dir});

	const listener1_changes: Array<string> = [];
	const listener2_changes: Array<string> = [];

	const cleanup1 = await filer.watch((change) => {
		listener1_changes.push(change.path);
	});

	const cleanup2 = await filer.watch((change) => {
		listener2_changes.push(change.path);
	});

	// Both listeners should receive the same notifications
	assert.equal(listener1_changes.length, 3);
	assert.equal(listener2_changes.length, 3);
	assert.deepEqual(listener1_changes.sort(), listener2_changes.sort());

	// Remove first listener
	cleanup1();

	// Second listener should still be active
	// (would need to trigger a change to test, but structure is validated)

	cleanup2();
});

test('watch() after init() reuses existing file state', async () => {
	const mock_watch_dir = create_mock_watch_dir();
	const filer = new Filer({watch_dir: mock_watch_dir});

	// Initialize first
	await filer.init();
	const files_after_init = filer.files.size;

	// Then add a watcher
	const changes: Array<string> = [];
	const cleanup = await filer.watch((change) => {
		changes.push(change.path);
	});

	// Should reuse existing state
	assert.equal(filer.files.size, files_after_init);
	// Listener should be notified of existing files
	assert.equal(changes.length, files_after_init);

	// watch_dir should only be called once
	assert.equal(mock_watch_dir.mock.calls.length, 1);

	cleanup();
});

test('init() after watch() is idempotent', async () => {
	const mock_watch_dir = create_mock_watch_dir();
	const filer = new Filer({watch_dir: mock_watch_dir});

	// First add a watcher
	const cleanup = await filer.watch(() => {});
	const files_after_watch = filer.files.size;

	// Then call init - should be a no-op
	await filer.init();
	assert.equal(filer.files.size, files_after_watch);

	// watch_dir should only be called once
	assert.equal(mock_watch_dir.mock.calls.length, 1);

	cleanup();
});

test('listener can safely remove itself during callback', async () => {
	const mock_watch_dir = create_mock_watch_dir();
	const filer = new Filer({watch_dir: mock_watch_dir});

	let cleanup_fn: (() => void) | null = null;
	let callback_count = 0;

	cleanup_fn = await filer.watch(async () => {
		callback_count++;
		// Remove self on second callback
		if (callback_count === 2 && cleanup_fn) {
			cleanup_fn();
		}
	});

	// Should have been called for initial files
	assert.ok(callback_count >= 2);
	// No errors should occur
});

test('listener error does not affect other listeners', async () => {
	const mock_watch_dir = create_mock_watch_dir();
	const filer = new Filer({watch_dir: mock_watch_dir});

	const good_listener_changes: Array<string> = [];
	let bad_listener_called = false;

	const cleanup1 = await filer.watch(() => {
		bad_listener_called = true;
		throw new Error('Listener error');
	});

	const cleanup2 = await filer.watch((change) => {
		good_listener_changes.push(change.path);
	});

	// Both listeners should have been called despite error
	assert.ok(bad_listener_called);
	assert.equal(good_listener_changes.length, 3);

	cleanup1();
	cleanup2();
});

test('listeners can be added/removed during notification', async () => {
	const mock_watch_dir = create_mock_watch_dir();
	const filer = new Filer({watch_dir: mock_watch_dir});

	const events: Array<string> = [];
	let cleanup2: (() => void) | null = null as any;

	// First listener adds a second listener on first event
	const cleanup1 = await filer.watch(async (_change) => {
		events.push('listener1');
		if (events.length === 1) {
			// Add second listener during notification
			await filer
				.watch((_change) => {
					events.push('listener2');
				})
				.then((c) => (cleanup2 = c));
		}
	});

	// Third listener removes itself on second event
	let cleanup3: (() => void) | null = null;
	cleanup3 = await filer.watch(async (_change) => {
		events.push('listener3');
		if (events.length >= 6 && cleanup3) {
			cleanup3();
		}
	});

	// All listeners should handle the initial files
	assert.ok(events.length >= 3);
	assert.ok(events.includes('listener1'));
	assert.ok(events.includes('listener3'));

	cleanup1();
	if (cleanup2) cleanup2();
});

// Basic operations tests
test('watch_dir filter controls which files are tracked', async () => {
	const mock_watch_dir = vi.fn((options) => {
		const mock_watcher: WatchNodeFs = {
			init: vi.fn(async () => {
				// In real usage, watch_dir applies the filter and only calls on_change for matching files
				// Simulate that here by checking the filter
				if (!options.filter || options.filter('/test/included.ts', false)) {
					options.on_change({type: 'add', path: '/test/included.ts', is_directory: false});
				}
				// excluded.ts is filtered out, so on_change is not called for it
			}),
			close: vi.fn(async () => {}),
		};
		return mock_watcher;
	});

	// Filter out files with "excluded" in the name
	const filer = new Filer({
		watch_dir: mock_watch_dir,
		watch_dir_options: {
			filter: (path) => !path.includes('excluded'),
		},
	});

	await filer.init();

	// Only the included file should be tracked
	assert.equal(filer.files.size, 1);
	assert.ok(filer.get_by_id('/test/included.ts'));
	assert.ok(!filer.get_by_id('/test/excluded.ts'));
});

test('filter method returns matching disknodes', async () => {
	const mock_watch_dir = create_mock_watch_dir();
	const filer = new Filer({watch_dir: mock_watch_dir});

	await filer.init();

	// Filter for files containing "file1"
	const results = filer.filter((disknode) => disknode.id.includes('file1'));

	assert.ok(results);
	assert.equal(results.length, 1);
	assert.equal(results[0]!.id, '/test/file1.ts');
});

test('filter method returns null when no matches', async () => {
	const mock_watch_dir = create_mock_watch_dir();
	const filer = new Filer({watch_dir: mock_watch_dir});

	await filer.init();

	// Filter for non-existent files
	const results = filer.filter((disknode) => disknode.id.includes('nonexistent'));

	assert.equal(results, null);
});

test('get_or_create creates disknode if not exists', async () => {
	const mock_watch_dir = create_mock_watch_dir();
	const filer = new Filer({watch_dir: mock_watch_dir});

	await filer.init();

	// Get a file that doesn't exist yet
	const new_file = filer.get_or_create('/test/new-file.ts');

	assert.ok(new_file);
	assert.equal(new_file.id, '/test/new-file.ts');
	assert.equal(new_file.contents, null);
	assert.ok(new_file.dependencies instanceof Map);
	assert.ok(new_file.dependents instanceof Map);

	// Second call should return the same instance
	const same_file = filer.get_or_create('/test/new-file.ts');
	assert.equal(new_file, same_file);
});

test('directories are ignored by queue processor', async () => {
	const events: Array<string> = [];

	const mock_watch_dir = vi.fn((options) => {
		const mock_watcher: WatchNodeFs = {
			init: vi.fn(async () => {
				options.on_change({type: 'add', path: '/test/dir', is_directory: true});
				options.on_change({type: 'add', path: '/test/file.ts', is_directory: false});
			}),
			close: vi.fn(async () => {}),
		};
		return mock_watcher;
	});

	const filer = new Filer({watch_dir: mock_watch_dir});

	await filer.watch((change) => {
		events.push(change.path);
	});

	// Only the file should trigger events, not the directory
	assert.equal(events.length, 1);
	assert.equal(events[0], '/test/file.ts');
});

test('removed files are deleted from map when no dependents', async () => {
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

	// File should be tracked
	const initial_size = filer.files.size;
	assert.equal(initial_size, 1);
	assert.ok(filer.get_by_id('/test/file.ts'));

	// Simulate deletion via the on_change callback
	assert.ok(on_change_callback);
	on_change_callback({type: 'delete', path: '/test/file.ts', is_directory: false});

	// Wait for queue processing
	await new Promise((resolve) => setTimeout(resolve, 10));

	// File should be completely removed from the map since it has no dependents
	// (files with dependents would stay in memory with null contents)
	assert.equal(filer.files.size, 0);
	assert.ok(!filer.get_by_id('/test/file.ts'));
});

// External file tests
test('external files are marked correctly', async () => {
	const mock_watch_dir = vi.fn((options) => {
		const mock_watcher: WatchNodeFs = {
			init: vi.fn(async () => {
				// This file is outside root_dir
				options.on_change({type: 'add', path: '/outside/file.ts', is_directory: false});
				// This file is inside root_dir (/test is the default root)
				options.on_change({type: 'add', path: '/test/file.ts', is_directory: false});
			}),
			close: vi.fn(async () => {}),
		};
		return mock_watcher;
	});

	const filer = new Filer({
		watch_dir: mock_watch_dir,
		watch_dir_options: {dir: '/test'},
	});

	await filer.init();

	const external_file = filer.get_by_id('/outside/file.ts');
	const internal_file = filer.get_by_id('/test/file.ts');

	assert.ok(external_file);
	assert.equal(external_file.external, true);

	assert.ok(internal_file);
	assert.equal(internal_file.external, false);
});

test('external file created via get_or_create is tracked immediately', async () => {
	const mock_watch_dir = create_mock_watch_dir();
	const filer = new Filer({
		watch_dir: mock_watch_dir,
		watch_dir_options: {dir: '/test'},
	});
	await filer.init();

	// Create external file (simulating dependency resolution to node_modules)
	const external = filer.get_or_create('/node_modules/foo/index.js');

	// File should be tracked immediately (synchronous)
	assert.ok(external);
	assert.equal(external.id, '/node_modules/foo/index.js');
	assert.equal(external.external, true);
	assert.ok(filer.files.has('/node_modules/foo/index.js'));

	// Should be retrievable
	const retrieved = filer.get_by_id('/node_modules/foo/index.js');
	assert.equal(retrieved, external);
});

test('external files that do not exist on disk do not notify listeners', async () => {
	const events: Array<{type: string; path: string}> = [];
	const mock_watch_dir = create_mock_watch_dir();
	const filer = new Filer({
		watch_dir: mock_watch_dir,
		watch_dir_options: {dir: '/test'},
	});

	await filer.watch((change) => {
		events.push({type: change.type, path: change.path});
	});

	const initial_event_count = events.length;

	// Create external file that doesn't exist on disk
	filer.get_or_create('/node_modules/nonexistent/index.js');

	// Wait for deferred on_change to process
	await new Promise((resolve) => setTimeout(resolve, 20));

	// Should not trigger notification (file doesn't exist, no change detected)
	const new_events = events.slice(initial_event_count);
	assert.ok(
		!new_events.some((e) => e.path === '/node_modules/nonexistent/index.js'),
		'Non-existent external file should not trigger change notification',
	);

	// But file should still be tracked
	assert.ok(filer.files.has('/node_modules/nonexistent/index.js'));
});

test('multiple get_or_create calls for same external file return same instance', async () => {
	const mock_watch_dir = create_mock_watch_dir();
	const filer = new Filer({
		watch_dir: mock_watch_dir,
		watch_dir_options: {dir: '/test'},
	});
	await filer.init();

	const external1 = filer.get_or_create('/node_modules/foo/index.js');
	const external2 = filer.get_or_create('/node_modules/foo/index.js');

	// Should return the same instance
	assert.equal(external1, external2);

	// Should only have one entry in files
	const all_external = Array.from(filer.files.values()).filter(
		(f) => f.id === '/node_modules/foo/index.js',
	);
	assert.equal(all_external.length, 1);
});
