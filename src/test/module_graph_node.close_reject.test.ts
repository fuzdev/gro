import { test, assert, vi } from 'vitest';

import { create_node_watcher } from '#lib/module_graph_node.ts';

const state = vi.hoisted(() => ({ count: 0, slow_closed: false }));

vi.mock('#lib/watch_dir.ts', () => ({
	watch_dir: () => {
		const n = state.count++;
		return {
			init: () => Promise.resolve(),
			close: async () => {
				if (n === 0) throw Error('close failed');
				await new Promise((r) => setTimeout(r, 50));
				state.slow_closed = true;
			}
		};
	}
}));

test('close settles every watch before rejecting with the first failure', async () => {
	const watcher = create_node_watcher({ watch: { dir: '/nonexistent', filter: null } });
	await watcher.watch(() => {});
	await watcher.watch(() => {});
	let error: unknown;
	try {
		await watcher.close();
	} catch (e) {
		error = e;
	}
	assert.instanceOf(error, Error);
	assert.strictEqual(error.message, 'close failed');
	assert.ok(state.slow_closed);
});
