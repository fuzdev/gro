import { describe, test, assert, vi, beforeEach, afterEach } from 'vitest';
import { EventEmitter } from 'node:events';
import { join } from 'node:path';

import { watch, type WatchDirOptions, type WatchEvent, type Watcher } from '#lib/watch.ts';

import { create_fs_sandbox, remove_fs_sandbox, type FsSandbox } from './fs_test_helpers.ts';

/**
 * Stands in for chokidar's `FSWatcher` as far as `watch.ts` uses one: an emitter of `ready`
 * and of the entry events, each with the entry's absolute path, and a `close`.
 * It watches nothing, so a test emits every event itself and the watcher's buffer takes each
 * as it's emitted, where a real watcher's events arrive on chokidar's own timers,
 * and nothing shows when the last of them has.
 */
class FakeChokidar extends EventEmitter {
	readonly root: string;

	constructor(root: string) {
		super();
		this.root = root;
		// chokidar's `ready` follows its scan, never emitted as the watcher is made
		setImmediate(() => this.emit('ready'));
	}

	// a promise, as chokidar's is
	async close(): Promise<void> {
		this.removeAllListeners();
	}
}

// the watchers `watch.ts` asked chokidar for, the newest last
const fakes: Array<FakeChokidar> = [];

vi.mock('chokidar', () => ({
	watch: (root: string) => {
		const fake = new FakeChokidar(root);
		fakes.push(fake);
		return fake;
	}
}));

let sandbox: FsSandbox;
const watchers: Array<Watcher> = [];

interface Started {
	events: AsyncIterator<WatchEvent>;
	/**
	 * Emits an `add` for each name, as chokidar reports a file made in the watched directory.
	 */
	add: (...names: Array<string>) => void;
}

const start = async (options?: WatchDirOptions): Promise<Started> => {
	// a real directory, which `watch.dir` lists and `lstat`s, with nothing ever made in it
	const watcher = await watch.dir(sandbox.dir, options);
	watchers.push(watcher);
	const fake = fakes.at(-1);
	assert.ok(fake, 'the watcher is over the fake');
	assert.strictEqual(fake.root, sandbox.root);
	return {
		events: watcher[Symbol.asyncIterator](),
		add: (...names) => {
			for (const name of names) fake.emit('add', join(fake.root, name));
		}
	};
};

const to_summary = (events: Array<WatchEvent>): Array<string> =>
	events.map((e) => (e.kind === 'rescan' ? `rescan ${e.path}` : `${e.kind} ${e.path}`));

/**
 * Reads `count` events that are already buffered, failing on a read that would wait.
 */
const read_buffered = async (
	events: AsyncIterator<WatchEvent>,
	count: number
): Promise<Array<WatchEvent>> => {
	const read: Array<WatchEvent> = [];
	for (let i = 0; i < count; i++) {
		let immediate: NodeJS.Immediate | undefined;
		// a buffered event's read is already resolved, so it settles before anything scheduled
		const waited = new Promise<'waited'>((resolve) => {
			immediate = setImmediate(() => resolve('waited'));
		});
		const result = await Promise.race([events.next(), waited]);
		clearImmediate(immediate);
		if (result === 'waited') {
			assert.fail(
				`nothing buffered at read ${i} of ${count}, the last read ${JSON.stringify(to_summary(read.slice(-3)))}`
			);
		}
		if (result.done) assert.fail(`the watcher ended at read ${i} of ${count}`);
		read.push(result.value);
	}
	return read;
};

const to_names = (count: number): Array<string> => Array.from({ length: count }, (_, i) => `f${i}`);

const to_adds = (names: Array<string>): Array<string> => names.map((name) => `add ${name}`);

const RESCAN: WatchEvent = { kind: 'rescan', path: '' };

const to_add = (path: string): WatchEvent => ({ kind: 'add', path, is_directory: false });

beforeEach(async () => {
	sandbox = await create_fs_sandbox();
});

afterEach(async () => {
	await Promise.all(watchers.splice(0).map((w) => w.close()));
	fakes.length = 0;
	remove_fs_sandbox(sandbox);
});

const bounds: Array<{ name: string; max_pending: number; options: WatchDirOptions | undefined }> = [
	{ name: '`max_pending: 1`', max_pending: 1, options: { max_pending: 1 } },
	{ name: '`max_pending: 3`', max_pending: 3, options: { max_pending: 3 } },
	{ name: 'the default `max_pending`', max_pending: 4096, options: undefined }
];

for (const { name, max_pending, options } of bounds) {
	describe(name, () => {
		const names = to_names(max_pending);

		test('`max_pending` events buffer without a `rescan`', async () => {
			const { events, add } = await start(options);
			add(...names);
			const read = await read_buffered(events, max_pending);
			assert.deepEqual(to_summary(read), to_adds(names));
			// nothing behind them: the next event is the next read
			add('end');
			assert.deepEqual(to_summary(await read_buffered(events, 1)), ['add end']);
		});

		test('one event past `max_pending` collapses them into one `rescan`', async () => {
			const { events, add } = await start(options);
			add(...names, 'over');
			assert.deepEqual(await read_buffered(events, 1), [RESCAN]);
			// one `rescan`, and none of the events it stands for behind it
			add('end');
			assert.deepEqual(to_summary(await read_buffered(events, 1)), ['add end']);
		});

		test('events after the overflow are dropped until the `rescan` is read', async () => {
			const { events, add } = await start(options);
			add(...names, 'over', 'dropped0', 'dropped1', ...names);
			assert.deepEqual(await read_buffered(events, 1), [RESCAN]);
			// once it's read the bound starts again, `max_pending` events and then one more
			add(...names);
			assert.deepEqual(to_summary(await read_buffered(events, max_pending)), to_adds(names));
			add(...names, 'over');
			assert.deepEqual(await read_buffered(events, 1), [RESCAN]);
			add('end');
			assert.deepEqual(to_summary(await read_buffered(events, 1)), ['add end']);
		});

		test('an event a waiting reader takes is never buffered', async () => {
			const { events, add } = await start(options);
			const waiting = events.next();
			add('taken', ...names);
			assert.deepEqual(await waiting, { done: false, value: to_add('taken') });
			// `max_pending` events behind the one taken, and no `rescan`
			assert.deepEqual(to_summary(await read_buffered(events, max_pending)), to_adds(names));
			add('end');
			assert.deepEqual(to_summary(await read_buffered(events, 1)), ['add end']);
		});
	});
}
