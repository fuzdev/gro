import { describe, test, assert, beforeEach, afterEach } from 'vitest';
import { spawn as node_spawn } from 'node:child_process';
import { getEventListeners } from 'node:events';
import {
	chmodSync,
	mkdirSync,
	readdirSync,
	readFileSync,
	readlinkSync,
	renameSync,
	rmSync,
	symlinkSync,
	unlinkSync,
	writeFileSync
} from 'node:fs';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

import { is_fs_error } from '#lib/fs_error.ts';
import { register_fs_handle } from '#lib/internal/fs_handles.ts';
import { paths } from '#lib/paths.ts';
import { watch, type WatchEvent, type Watcher } from '#lib/watch.ts';

import {
	assert_fs_error_rejects,
	create_fs_sandbox,
	remove_fs_sandbox,
	type FsSandbox
} from './fs_test_helpers.ts';

// how long a test waits for an event before failing, a bound, never an assertion
const EVENT_TIMEOUT_MS = 10_000;

/**
 * Reads events until one matches, failing past the timeout.
 * @returns every event read, the match last
 */
const read_until = async (
	iterator: AsyncIterator<WatchEvent>,
	matches: (event: WatchEvent) => boolean
): Promise<Array<WatchEvent>> => {
	const events: Array<WatchEvent> = [];
	const deadline = Date.now() + EVENT_TIMEOUT_MS;
	for (;;) {
		let timer: ReturnType<typeof setTimeout> | undefined;
		const timeout = new Promise<'timeout'>((resolve) => {
			timer = setTimeout(() => resolve('timeout'), Math.max(0, deadline - Date.now()));
		});
		const result = await Promise.race([iterator.next(), timeout]);
		clearTimeout(timer);
		if (result === 'timeout') {
			assert.fail(`no matching event within ${EVENT_TIMEOUT_MS}ms, read ${JSON.stringify(events)}`);
		}
		if (result.done) assert.fail(`the watcher ended, read ${JSON.stringify(events)}`);
		events.push(result.value);
		if (matches(result.value)) return events;
	}
};

const is_event =
	(kind: WatchEvent['kind'], path: string) =>
	(event: WatchEvent): boolean =>
		event.kind === kind && event.path === path;

const to_summary = (events: Array<WatchEvent>): Array<string> =>
	events.map((e) =>
		e.kind === 'rescan' ? `rescan ${e.path}` : `${e.kind} ${e.path}${e.is_directory ? '/' : ''}`
	);

/**
 * Counts the inotify watches this process holds, from `/proc`.
 * @returns the count, or `null` where `/proc` has no inotify descriptors to read
 */
const count_inotify_watches = (): number | null => {
	let count: number | null = null;
	let fds: Array<string>;
	try {
		fds = readdirSync('/proc/self/fd');
	} catch {
		return null;
	}
	for (const fd of fds) {
		try {
			if (readlinkSync(`/proc/self/fd/${fd}`) !== 'anon_inode:inotify') continue;
			const info = readFileSync(`/proc/self/fdinfo/${fd}`, 'utf8');
			count = (count ?? 0) + info.split('\n').filter((l) => l.startsWith('inotify ')).length;
		} catch {
			// a descriptor closed while it was read
		}
	}
	return count;
};

let sandbox: FsSandbox;
const watchers: Array<Watcher> = [];

const start = async (...args: Parameters<typeof watch.dir>): Promise<AsyncIterator<WatchEvent>> => {
	const watcher = await watch.dir(...args);
	watchers.push(watcher);
	return watcher[Symbol.asyncIterator]();
};

beforeEach(async () => {
	sandbox = await create_fs_sandbox();
});

afterEach(async () => {
	await Promise.all(watchers.splice(0).map((w) => w.close()));
	remove_fs_sandbox(sandbox);
});

describe('events', () => {
	test('reports changes after the watch is in place, none for what was there', async () => {
		writeFileSync(join(sandbox.root, 'before.txt'), '');
		mkdirSync(join(sandbox.root, 'sub'));
		writeFileSync(join(sandbox.root, 'sub', 'before.txt'), '');
		const events = await start(sandbox.dir, { recursive: true });

		writeFileSync(join(sandbox.root, 'sub', 'a.txt'), '1');
		const read = await read_until(events, is_event('add', 'sub/a.txt'));
		// nothing that was there before, only the new file
		assert.deepEqual(to_summary(read), ['add sub/a.txt']);

		writeFileSync(join(sandbox.root, 'sub', 'a.txt'), '22');
		await read_until(events, is_event('update', 'sub/a.txt'));
		unlinkSync(join(sandbox.root, 'sub', 'a.txt'));
		const deleted = await read_until(events, is_event('delete', 'sub/a.txt'));
		assert.deepEqual(deleted.at(-1), { kind: 'delete', path: 'sub/a.txt', is_directory: false });
	});

	test('paths are relative to the handle, `/`-separated', async () => {
		mkdirSync(join(sandbox.root, 'a', 'b'), { recursive: true });
		const events = await start(sandbox.dir.dir('a'), { recursive: true });
		writeFileSync(join(sandbox.root, 'a', 'b', 'c.txt'), '');
		const read = await read_until(events, (e) => e.kind === 'add');
		assert.deepEqual(read, [{ kind: 'add', path: 'b/c.txt', is_directory: false }]);
	});

	test('a directory created and filled is an `add`, then its entries', async () => {
		const events = await start(sandbox.dir, { recursive: true });
		mkdirSync(join(sandbox.root, 'n', 'm'), { recursive: true });
		writeFileSync(join(sandbox.root, 'n', 'g.txt'), '');
		writeFileSync(join(sandbox.root, 'n', 'm', 'h.txt'), '');
		const read = await read_until(events, is_event('add', 'n/m/h.txt'));
		const summary = to_summary(read);
		assert.include(summary, 'add n/');
		assert.include(summary, 'add n/g.txt');
		assert.include(summary, 'add n/m/');
		assert.isBelow(summary.indexOf('add n/'), summary.indexOf('add n/g.txt'));
	});

	test('a rename is a `delete` and an `add`', async () => {
		writeFileSync(join(sandbox.root, 'old.txt'), '');
		const events = await start(sandbox.dir);
		renameSync(join(sandbox.root, 'old.txt'), join(sandbox.root, 'new.txt'));
		const read = to_summary(
			await read_until(events, (e) => e.kind !== 'rescan' && e.path === 'new.txt')
		);
		const rest = read.includes('delete old.txt')
			? read
			: to_summary(await read_until(events, is_event('delete', 'old.txt')));
		assert.include(read.concat(rest), 'add new.txt');
		assert.include(read.concat(rest), 'delete old.txt');
	});

	test('without `recursive`, only the directory’s own entries', async () => {
		mkdirSync(join(sandbox.root, 'sub'));
		const events = await start(sandbox.dir);
		writeFileSync(join(sandbox.root, 'sub', 'deep.txt'), '');
		mkdirSync(join(sandbox.root, 'sub2'));
		writeFileSync(join(sandbox.root, 'top.txt'), '');
		const read = to_summary(await read_until(events, is_event('add', 'top.txt')));
		assert.notInclude(read, 'add sub/deep.txt');
		assert.include(read, 'add sub2/');
		rmSync(join(sandbox.root, 'sub'), { recursive: true });
		const deleted = to_summary(await read_until(events, is_event('delete', 'sub')));
		assert.deepEqual(deleted, ['delete sub/']);
	});

	test('events are frozen null-prototype records', async () => {
		const events = await start(sandbox.dir);
		writeFileSync(join(sandbox.root, 'a.txt'), '');
		const [event] = await read_until(events, is_event('add', 'a.txt'));
		assert.ok(Object.isFrozen(event));
		assert.isNull(Object.getPrototypeOf(event));
	});
});

describe('symlinks', () => {
	test('a symlink is an entry, never followed', async () => {
		writeFileSync(join(sandbox.outside, 'o.txt'), '1');
		symlinkSync(sandbox.outside, join(sandbox.root, 'out_link'));
		symlinkSync('.', join(sandbox.root, 'self_link'));
		const events = await start(sandbox.dir, { recursive: true });

		// a change beyond the link, then one inside, read up to the inside one
		writeFileSync(join(sandbox.outside, 'o.txt'), '2');
		writeFileSync(join(sandbox.outside, 'p.txt'), '');
		writeFileSync(join(sandbox.root, 'inside.txt'), '');
		const read = to_summary(await read_until(events, is_event('add', 'inside.txt')));
		assert.deepEqual(read, ['add inside.txt']);

		symlinkSync(join(sandbox.outside, 'o.txt'), join(sandbox.root, 'file_link'));
		const added = await read_until(events, (e) => e.kind !== 'rescan' && e.path === 'file_link');
		assert.deepEqual(added.at(-1), { kind: 'add', path: 'file_link', is_directory: false });
		writeFileSync(join(sandbox.outside, 'o.txt'), '3');
		writeFileSync(join(sandbox.root, 'inside2.txt'), '');
		const after = to_summary(await read_until(events, is_event('add', 'inside2.txt')));
		assert.deepEqual(after, ['add inside2.txt']);
	});
});

describe('a symlink swapped in', () => {
	test('a watched subdirectory replaced by a symlink reports nothing through it', async () => {
		mkdirSync(join(sandbox.root, 'sub'));
		writeFileSync(join(sandbox.outside, 'o.txt'), '');
		const events = await start(sandbox.dir, { recursive: true });
		// removed and replaced at once, before chokidar sees the directory go
		rmSync(join(sandbox.root, 'sub'), { recursive: true });
		symlinkSync(sandbox.outside, join(sandbox.root, 'sub'));
		await read_until(events, is_event('update', 'sub'));
		writeFileSync(join(sandbox.outside, 'beyond.txt'), '');
		// a second watcher started after the swap never follows the link, so it shows the marker arrived
		const witness = await start(sandbox.dir, { recursive: true });
		writeFileSync(join(sandbox.root, 'marker.txt'), '');
		await read_until(witness, is_event('add', 'marker.txt'));
		const read = to_summary(await read_until(events, is_event('add', 'marker.txt')));
		for (const event of read) assert.notMatch(event, /^\w+ sub\//, JSON.stringify(read));
	});

	test('subdirectories swapped for symlinks as chokidar scans them report nothing through them', async () => {
		writeFileSync(join(sandbox.outside, 'beyond.txt'), '');
		// the race is chokidar's `lstat` then `readdir` of a directory it's scanning,
		// so it's tried over many directories, each round a watch of its own
		for (let round = 0; round < 6; round++) {
			const name = `r${round}`;
			mkdirSync(join(sandbox.root, name));
			const incoming = join(sandbox.base, `incoming${round}`);
			for (let i = 0; i < 40; i++) {
				mkdirSync(join(incoming, `s${i}`, 'b', 'c'), { recursive: true });
				writeFileSync(join(incoming, `s${i}`, 'b', 'c', 'x'), '');
			}
			const events = await start(sandbox.dir.dir(name), { recursive: true });
			renameSync(incoming, join(sandbox.root, name, 'in'));
			// swapped once chokidar has seen the directories, as it reads them
			await new Promise((resolve) => setTimeout(resolve, 2));
			for (let i = 0; i < 40; i += 3) {
				const path = join(sandbox.root, name, 'in', `s${i}`);
				renameSync(path, join(sandbox.base, `gone${round}_${i}`));
				symlinkSync(sandbox.outside, path);
			}
			writeFileSync(join(sandbox.root, name, 'marker.txt'), '');
			const read = to_summary(await read_until(events, is_event('add', 'marker.txt')));
			for (const event of read)
				assert.notMatch(event, /beyond\.txt|secret\.txt/, JSON.stringify(read));
		}
	});

	test('chokidar never descends into a subdirectory replaced by a symlink', async ({ skip }) => {
		mkdirSync(join(sandbox.root, 'sub'));
		for (let i = 0; i < 10; i++) mkdirSync(join(sandbox.outside, `d${i}`));
		const events = await start(sandbox.dir, { recursive: true });
		const before = count_inotify_watches();
		if (before === null) skip('counts inotify watches through `/proc`');
		rmSync(join(sandbox.root, 'sub'), { recursive: true });
		symlinkSync(sandbox.outside, join(sandbox.root, 'sub'));
		await read_until(events, is_event('update', 'sub'));
		const witness = await start(sandbox.dir);
		writeFileSync(join(sandbox.root, 'marker.txt'), '');
		await read_until(witness, is_event('add', 'marker.txt'));
		await read_until(events, is_event('add', 'marker.txt'));
		// the witness's one watch, and at most one on the link's target, never one beneath it
		assert.isAtMost(count_inotify_watches()! - before!, 2);
	});

	/**
	 * Reads events, making a change through `poke` before each read, until the watch ends,
	 * and asserts it ended with `not_found` having reported none of the changes.
	 */
	const assert_ends_unreported = async (
		events: AsyncIterator<WatchEvent>,
		poke: (index: number) => void
	): Promise<void> => {
		const read: Array<WatchEvent> = [];
		let error: unknown;
		const deadline = Date.now() + EVENT_TIMEOUT_MS;
		try {
			for (let i = 0; ; i++) {
				poke(i);
				const result = await Promise.race([
					events.next(),
					new Promise<null>((resolve) => setTimeout(() => resolve(null), 100))
				]);
				if (result === null) {
					assert.isBelow(Date.now(), deadline, 'the watch never ended');
					continue;
				}
				if (result.done) break;
				read.push(result.value);
			}
		} catch (e) {
			error = e;
		}
		assert.ok(is_fs_error(error, 'not_found'), String(error));
		for (const event of to_summary(read)) assert.notMatch(event, /poked/);
	};

	test('the watched directory replaced by a symlink ends with `not_found`', async () => {
		mkdirSync(join(sandbox.root, 'w'));
		writeFileSync(join(sandbox.root, 'w', 'a.txt'), '');
		const events = await start(sandbox.dir.dir('w'), { recursive: true });
		renameSync(join(sandbox.root, 'w'), join(sandbox.root, 'moved'));
		symlinkSync(sandbox.outside, join(sandbox.root, 'w'));
		await assert_ends_unreported(events, (i) =>
			writeFileSync(join(sandbox.outside, `poked${i}.txt`), '')
		);
	});

	test('the watched directory replaced by another directory ends with `not_found`', async () => {
		mkdirSync(join(sandbox.root, 'w'));
		writeFileSync(join(sandbox.root, 'w', 'a.txt'), '');
		const events = await start(sandbox.dir.dir('w'), { recursive: true });
		renameSync(join(sandbox.root, 'w'), join(sandbox.root, 'moved'));
		mkdirSync(join(sandbox.root, 'w'));
		await assert_ends_unreported(events, (i) =>
			writeFileSync(join(sandbox.root, 'w', `poked${i}.txt`), '')
		);
	});
});

describe('the handle', () => {
	test('a read-only handle watches, and nothing beside it is reported', async () => {
		mkdirSync(join(sandbox.root, 'mine'));
		mkdirSync(join(sandbox.root, 'sibling'));
		const events = await start(sandbox.dir.readonly().dir('mine'), { recursive: true });
		writeFileSync(join(sandbox.root, 'sibling', 's.txt'), '');
		writeFileSync(join(sandbox.root, 'top.txt'), '');
		writeFileSync(join(sandbox.root, 'mine', 'm.txt'), '');
		const read = to_summary(await read_until(events, is_event('add', 'm.txt')));
		assert.deepEqual(read, ['add m.txt']);
	});

	test('a missing directory or a file rejects as listing it does', async () => {
		mkdirSync(join(sandbox.root, 'gone'));
		const gone = sandbox.dir.dir('gone');
		rmSync(join(sandbox.root, 'gone'), { recursive: true });
		await assert_fs_error_rejects(watch.dir(gone), 'not_found');
	});

	test('an unreadable directory rejects with `permission_denied`', async ({ skip }) => {
		if (process.getuid?.() === 0) skip('root reads every directory');
		mkdirSync(join(sandbox.root, 'locked'));
		const locked = sandbox.dir.dir('locked');
		chmodSync(join(sandbox.root, 'locked'), 0);
		try {
			await assert_fs_error_rejects(watch.dir(locked), 'permission_denied');
		} finally {
			chmodSync(join(sandbox.root, 'locked'), 0o755);
		}
	});

	test('a handle whose directory was replaced by a symlink rejects with `outside_root`', async () => {
		mkdirSync(join(sandbox.root, 'w'));
		const handle = sandbox.dir.dir('w');
		renameSync(join(sandbox.root, 'w'), join(sandbox.root, 'moved'));
		symlinkSync(sandbox.outside, join(sandbox.root, 'w'));
		await assert_fs_error_rejects(watch.dir(handle), 'outside_root');
	});

	test('a `dir` this copy of fs never made is a `TypeError` naming it, before anything is read', async () => {
		let listed = false;
		const list = async function* () {
			listed = true;
			yield* [];
		};
		const real = sandbox.dir;
		const forged: Array<unknown> = [
			null,
			undefined,
			'dir',
			sandbox.root,
			{},
			{ path: 'relative' },
			// shaped like a handle, its listing opening, so only the registry refuses these
			{ path: sandbox.root, list },
			{ path: '/', list },
			// a real handle's prototype, as another copy of Gro's handle would have, and its own fields
			Object.freeze(Object.create(Object.getPrototypeOf(real), { path: { value: sandbox.root } })),
			Object.freeze({ ...real, list }),
			watch
		];
		for (const [i, dir] of forged.entries()) {
			let error: unknown;
			try {
				await watch.dir(dir as any);
			} catch (e) {
				error = e;
			}
			assert.instanceOf(error, TypeError, `forged[${i}]`);
			assert.match(error.message, /`watch\.dir`'s `dir`.*fs directory handle/);
		}
		assert.isFalse(listed);
		// a real handle, read-only or not, still watches
		mkdirSync(join(sandbox.root, 'sub'));
		for (const dir of [real, real.readonly(), real.dir('sub').readonly()]) {
			const watcher = await watch.dir(dir);
			watchers.push(watcher);
		}
	});
});

describe('rescan', () => {
	test('a subdirectory that can’t be read is a `rescan` of it', async ({ skip }) => {
		if (process.getuid?.() === 0) skip('root reads every directory');
		mkdirSync(join(sandbox.root, 'locked'));
		chmodSync(join(sandbox.root, 'locked'), 0);
		try {
			const events = await start(sandbox.dir, { recursive: true });
			const read = await read_until(events, (e) => e.kind === 'rescan');
			assert.deepEqual(read, [{ kind: 'rescan', path: 'locked' }]);
		} finally {
			chmodSync(join(sandbox.root, 'locked'), 0o755);
		}
	});

	test('a subdirectory locked as it’s watched is a `rescan` of it', async ({ skip }) => {
		if (process.getuid?.() === 0) skip('root reads every directory');
		mkdirSync(join(sandbox.root, 'locked', 'inner'), { recursive: true });
		const events = await start(sandbox.dir, { recursive: true });
		chmodSync(join(sandbox.root, 'locked'), 0);
		try {
			const read = await read_until(events, (e) => e.kind === 'rescan');
			// chokidar's error names `locked/locked`, which doesn't exist
			assert.deepEqual(read.at(-1), { kind: 'rescan', path: 'locked' });
		} finally {
			chmodSync(join(sandbox.root, 'locked'), 0o755);
		}
	});

	test('`max_pending` events buffer without a `rescan`', async () => {
		const events = await start(sandbox.dir, { max_pending: 3 });
		const witness = await start(sandbox.dir);
		for (const name of ['a', 'b', 'c']) writeFileSync(join(sandbox.root, name), '');
		await read_until(witness, is_event('add', 'c'));
		const read = to_summary(await read_until(events, is_event('add', 'c')));
		assert.sameMembers(read, ['add a', 'add b', 'add c']);
	});

	test('one event past `max_pending` is a `rescan`', async () => {
		const events = await start(sandbox.dir, { max_pending: 3 });
		const witness = await start(sandbox.dir);
		for (const name of ['a', 'b', 'c', 'd']) writeFileSync(join(sandbox.root, name), '');
		await read_until(witness, is_event('add', 'd'));
		assert.deepEqual(await events.next(), { done: false, value: { kind: 'rescan', path: '' } });
	});

	test('events past `max_pending` are dropped for one `rescan` of everything', async () => {
		const events = await start(sandbox.dir, { max_pending: 3 });
		// a second watcher, unbounded, shows when the events have arrived
		const witness = await start(sandbox.dir);
		const count = 40;
		for (let i = 0; i < count; i++) writeFileSync(join(sandbox.root, `f${i}.txt`), '');
		writeFileSync(join(sandbox.root, 'last.txt'), '');
		await read_until(witness, is_event('add', 'last.txt'));

		const first = await events.next();
		assert.deepEqual(first, { done: false, value: { kind: 'rescan', path: '' } });
		// once it's read, events queue again, read after they've arrived so none meets a waiting reader;
		// the batch's last events may still be arriving behind the rescan, the witness no sign of them,
		// and overflow again, so a file is made until its `add` is read
		for (let i = 0; i <= count; i++) {
			const name = `after${i}.txt`;
			writeFileSync(join(sandbox.root, name), '');
			await read_until(witness, is_event('add', name));
			const read = to_summary(
				await read_until(events, (e) => e.kind === 'rescan' || is_event('add', name)(e))
			);
			for (const event of read.slice(0, -1)) {
				assert.match(event, /^add (f\d+|last|after\d+)\.txt$/);
			}
			if (read.at(-1) === `add ${name}`) return;
		}
		assert.fail('every file made after the `rescan` was read was dropped');
	});
});

describe('ending', () => {
	test('`signal` ends the iteration', async () => {
		const controller = new AbortController();
		const watcher = await watch.dir(sandbox.dir, { signal: controller.signal });
		watchers.push(watcher);
		const ended = (async () => {
			const seen: Array<WatchEvent> = [];
			for await (const event of watcher) seen.push(event);
			return seen;
		})();
		controller.abort();
		assert.deepEqual(await ended, []);
	});

	test('a `signal` already aborted rejects with its reason', async () => {
		const reason = new Error('stop');
		let error: unknown;
		try {
			await watch.dir(sandbox.dir, { signal: AbortSignal.abort(reason) });
		} catch (e) {
			error = e;
		}
		assert.strictEqual(error, reason);
	});

	test('`close` drops what wasn’t read and ends the iteration', async () => {
		const watcher = await watch.dir(sandbox.dir);
		watchers.push(watcher);
		const witness = await start(sandbox.dir);
		writeFileSync(join(sandbox.root, 'a.txt'), '');
		await read_until(witness, is_event('add', 'a.txt'));
		await watcher.close();
		const iterator = watcher[Symbol.asyncIterator]();
		assert.deepEqual(await iterator.next(), { done: true, value: undefined });
	});

	test('the watched directory removed ends with `not_found`, after what was queued', async () => {
		mkdirSync(join(sandbox.root, 'w'));
		writeFileSync(join(sandbox.root, 'w', 'a.txt'), '');
		const events = await start(sandbox.dir.dir('w'));
		const witness = await start(sandbox.dir);
		rmSync(join(sandbox.root, 'w'), { recursive: true });
		// read once it's gone, so what was queued meets no waiting reader
		await read_until(witness, is_event('delete', 'w'));
		const read: Array<WatchEvent> = [];
		let error: unknown;
		try {
			for (;;) {
				const result = await events.next();
				if (result.done) break;
				read.push(result.value);
			}
		} catch (e) {
			error = e;
		}
		assert.ok(is_fs_error(error, 'not_found'), String(error));
		assert.deepEqual(to_summary(read), ['delete a.txt']);
		assert.deepEqual(await events.next(), { done: true, value: undefined });
	});

	test('the watched directory removed before the watch is in place still settles', async () => {
		// removals from just after the listing to well into chokidar's scan
		for (const delay of [-1, 2, 5, 10, 20]) {
			const name = `w${delay}`;
			const w = join(sandbox.root, name);
			mkdirSync(w);
			// enough to scan that the removal lands while chokidar is still scanning
			for (let i = 0; i < 200; i++) mkdirSync(join(w, `d${i}`));
			const real = sandbox.dir.dir(name);
			// the handle's listing, which `watch.dir` reads first, then removes the directory,
			// registered as `fs.ts` registers a handle, so `watch.dir` takes it
			const handle = {
				path: real.path,
				async *list(path: string) {
					try {
						yield* real.list(path);
					} finally {
						const remove = () => rmSync(w, { recursive: true });
						if (delay < 0) setImmediate(remove);
						else setTimeout(remove, delay);
					}
				}
			};
			register_fs_handle(handle, () => assert.fail('unused'));
			let timer: ReturnType<typeof setTimeout> | undefined;
			const timeout = new Promise<'timeout'>((resolve) => {
				timer = setTimeout(() => resolve('timeout'), EVENT_TIMEOUT_MS);
			});
			// rejected, or resolved first and then ended by the removal
			const ended = watch.dir(handle as any, { recursive: true }).then(async (watcher) => {
				watchers.push(watcher);
				for await (const _event of watcher);
			});
			const outcome = await Promise.race([
				ended.then(
					() => 'done',
					(e: unknown) => e
				),
				timeout
			]);
			clearTimeout(timer);
			assert.ok(is_fs_error(outcome, 'not_found'), `${delay}ms: ${String(outcome)}`);
		}
	});

	test('a watcher is read once', async () => {
		const watcher = await watch.dir(sandbox.dir);
		watchers.push(watcher);
		watcher[Symbol.asyncIterator]();
		assert.throws(() => watcher[Symbol.asyncIterator](), TypeError);
	});

	const run_child = (mode: string): Promise<{ code: number | null; output: string }> =>
		new Promise((resolve, reject) => {
			const script = join(sandbox.base, 'child.ts');
			writeFileSync(
				script,
				`import {rmSync, writeFileSync} from 'node:fs';
import {fs} from ${JSON.stringify(pathToFileURL(join(paths.lib, 'fs.ts')).href)};
import {watch} from ${JSON.stringify(pathToFileURL(join(paths.lib, 'watch.ts')).href)};
const controller = new AbortController();
const watcher = await watch.dir(await fs.open_dir(${JSON.stringify(sandbox.root)}), {
	recursive: true,
	signal: controller.signal
});
const mode = ${JSON.stringify(mode)};
if (mode === 'abort') controller.abort();
if (mode === 'break') writeFileSync(${JSON.stringify(join(sandbox.root, 'x.txt'))}, '');
if (mode === 'removed') rmSync(${JSON.stringify(sandbox.root)}, {recursive: true});
try {
	for await (const event of watcher) {
		if (mode === 'break') break;
	}
} catch (error) {
	if (mode !== 'removed' || error.kind !== 'not_found') throw error;
}
console.log('ended');
`
			);
			const child = node_spawn(process.execPath, [script], { stdio: ['ignore', 'pipe', 'pipe'] });
			let output = '';
			child.stdout.on('data', (d) => (output += d));
			child.stderr.on('data', (d) => (output += d));
			const timer = setTimeout(() => child.kill('SIGKILL'), EVENT_TIMEOUT_MS);
			child.on('error', reject);
			child.on('close', (code) => {
				clearTimeout(timer);
				resolve({ code, output });
			});
		});

	test('nothing keeps the process alive after `signal` ends the watch', async () => {
		const { code, output } = await run_child('abort');
		assert.strictEqual(code, 0, output);
		assert.strictEqual(output.trim(), 'ended');
	});

	test('nothing keeps the process alive after leaving the loop', async () => {
		const { code, output } = await run_child('break');
		assert.strictEqual(code, 0, output);
		assert.strictEqual(output.trim(), 'ended');
	});

	test('nothing keeps the process alive after the watched directory is removed', async () => {
		const { code, output } = await run_child('removed');
		assert.strictEqual(code, 0, output);
		assert.strictEqual(output.trim(), 'ended');
	});
});

describe('the module', () => {
	test('`watch`, a watcher, and an iterator are frozen; prototypes null-prototype', async () => {
		assert.ok(Object.isFrozen(watch));
		assert.isNull(Object.getPrototypeOf(watch));
		const watcher = await watch.dir(sandbox.dir);
		watchers.push(watcher);
		const iterator = watcher[Symbol.asyncIterator]();
		for (const object of [watcher, iterator]) {
			assert.ok(Object.isFrozen(object));
			const proto = Object.getPrototypeOf(object);
			assert.ok(Object.isFrozen(proto));
			assert.isNull(Object.getPrototypeOf(proto));
			assert.isUndefined((object as any).then);
		}
	});

	test('a watcher comes only from the module, and its methods refuse other receivers', async () => {
		const watcher = await watch.dir(sandbox.dir);
		watchers.push(watcher);
		const proto = Object.getPrototypeOf(watcher);
		assert.throws(() => new proto.constructor(Symbol('forged'), null, null), TypeError);
		assert.throws(() => proto.close.call({}), TypeError);
		const iterator = watcher[Symbol.asyncIterator]();
		assert.throws(() => Object.getPrototypeOf(iterator).next.call({}), TypeError);
	});

	test('options are read by own property', async () => {
		// an inherited invalid value is never read
		const options = Object.create({ recursive: 'yes', max_pending: 0, signal: {} });
		const watcher = await watch.dir(sandbox.dir, options);
		watchers.push(watcher);
	});

	test('iterator results are frozen null-prototype records', async () => {
		const events = await start(sandbox.dir);
		writeFileSync(join(sandbox.root, 'a.txt'), '');
		const result = await events.next();
		assert.ok(Object.isFrozen(result));
		assert.isNull(Object.getPrototypeOf(result));
	});

	test('closing removes the `signal` listener', async () => {
		const controller = new AbortController();
		const watcher = await watch.dir(sandbox.dir, { signal: controller.signal });
		assert.strictEqual(getEventListeners(controller.signal, 'abort').length, 1);
		await watcher.close();
		assert.strictEqual(getEventListeners(controller.signal, 'abort').length, 0);
	});

	test('invalid options are a `TypeError`', async () => {
		for (const options of [
			'x',
			{ recursive: 'yes' },
			{ signal: {} },
			{ max_pending: 0 },
			{ max_pending: 1.5 }
		]) {
			let error: unknown;
			try {
				await watch.dir(sandbox.dir, options as any);
			} catch (e) {
				error = e;
			}
			assert.instanceOf(error, TypeError, JSON.stringify(options));
		}
	});
});
