import { vi, type Mock } from 'vitest';

import type { watch_dir, WatchNodeFs } from '#lib/internal/watch_dir.ts';
import type { Disknode } from '#lib/internal/disknode.ts';

// Create a simple mock watch_dir that simulates file discovery
export const create_mock_watch_dir = (): Mock<typeof watch_dir> => {
	return vi.fn((options) => {
		const mock_watcher: WatchNodeFs = {
			init: vi.fn(async () => {
				// Simulate discovering files via on_change callbacks
				options.on_change({ type: 'add', path: '/test/file1.ts', is_directory: false });
				options.on_change({ type: 'add', path: '/test/file2.ts', is_directory: false });
				options.on_change({ type: 'add', path: '/test/lib/helper.ts', is_directory: false });
			}),
			close: vi.fn(async () => {})
		};
		return mock_watcher;
	});
};

export const create_test_disknode = (id: string, contents: string | null = null): Disknode => ({
	id,
	contents,
	external: false,
	content_hash: null,
	dependents: new Map(),
	dependencies: new Map()
});
