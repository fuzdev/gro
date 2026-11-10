import { describe, test, expect, assert, vi } from 'vitest';
import type { PathId } from '@fuzdev/fuz_util/path.ts';

import {
	change_reaches_server,
	start_watched_server,
	type WatchedServerOptions
} from '#lib/internal/server_restarts.ts';
import type { FileChange, FileChangeCallback, ModuleGraph } from '#lib/module_graph.ts';
import type { SpawnChild, SpawnResult } from '#lib/spawn.ts';
import { SpawnError } from '#lib/spawn_error.ts';

import { create_mock_logger } from '../test_helpers.ts';
import { create_mock_child, create_mock_spawn } from '../spawn_mocks.ts';

const ENTRY = '/p/src/server.ts';
const DEP = '/p/src/dep.ts';
const DEEP = '/p/src/deep.ts';
const OTHER = '/p/src/other.ts';

/**
 * A graph of `ENTRY` <- `DEP` <- `DEEP` plus an unrelated `OTHER`, delivering an initial `add`
 * for each when watched, its changes emitted by hand.
 */
const create_graph = () => {
	const importers: Record<PathId, Array<PathId>> = { [DEP]: [ENTRY], [DEEP]: [DEP] };
	const listeners: Set<FileChangeCallback> = new Set();
	let unwatched = 0;
	const graph: Pick<ModuleGraph, 'dependents' | 'watch'> = {
		dependents: async (id) => {
			const found: Set<PathId> = new Set();
			const stack = [id];
			while (stack.length) {
				for (const importer of importers[stack.pop()!] ?? []) {
					if (found.has(importer)) continue;
					found.add(importer);
					stack.push(importer);
				}
			}
			return found;
		},
		watch: async (on_change) => {
			for (const id of [ENTRY, DEP, DEEP, OTHER]) on_change({ type: 'add', id });
			listeners.add(on_change);
			return async () => {
				unwatched++;
				listeners.delete(on_change);
			};
		}
	};
	return {
		graph,
		importers,
		emit: (change: FileChange) => {
			for (const listener of listeners) listener(change);
		},
		get unwatched() {
			return unwatched;
		}
	};
};

/**
 * A mock spawn whose children end only when disposed or `end`ed.
 */
const create_spawn = () => {
	const mock = create_mock_spawn();
	mock.available = () => true;
	const children: Array<SpawnChild & { end: (result: SpawnResult) => void }> = [];
	mock.create_child = () => {
		let end!: (result: SpawnResult) => void;
		const closed = new Promise<SpawnResult>((r) => {
			end = r;
		});
		const child = Object.assign(
			create_mock_child({
				pid: children.length,
				closed,
				[Symbol.asyncDispose]: async () => {
					end({ kind: 'signaled', signal: 'SIGTERM' });
					await closed;
				}
			}),
			{ end }
		);
		children.push(child);
		return child;
	};
	return { mock, children, program: mock.program('node') };
};

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

/**
 * Makes each child's stop take 60ms, so a restart stays in flight while a later change's
 * timer fires and joins it.
 */
const slow_stops = (mock: ReturnType<typeof create_spawn>['mock']): void => {
	const create = mock.create_child!;
	mock.create_child = (call) => {
		const child = create(call);
		const dispose = child[Symbol.asyncDispose];
		child[Symbol.asyncDispose] = async () => {
			await wait(60);
			await dispose();
		};
		return child;
	};
};

const to_options = (
	overrides: Partial<WatchedServerOptions> & Pick<WatchedServerOptions, 'program'>
): WatchedServerOptions => ({
	args: [ENTRY],
	entry_id: ENTRY,
	graph: null,
	log: create_mock_logger(),
	label: '[test]',
	cwd: '/p',
	restart_delay: 10,
	...overrides
});

/**
 * Polls `check` until it's true, failing after a deadline rather than sleeping a fixed time.
 */
const until = async (check: () => boolean, deadline_ms = 2000): Promise<void> => {
	const start = Date.now();
	while (!check()) {
		if (Date.now() - start > deadline_ms) throw Error('timed out');
		await wait(5);
	}
};

describe('change_reaches_server', () => {
	test('the entry, a direct import, and a transitive import reach it', async () => {
		const { graph } = create_graph();
		for (const id of [ENTRY, DEP, DEEP]) {
			expect(await change_reaches_server(graph, ENTRY, { type: 'update', id })).toBe(true);
		}
		expect(await change_reaches_server(graph, ENTRY, { type: 'delete', id: DEEP })).toBe(true);
	});

	test('a module outside the import closure does not', async () => {
		const { graph } = create_graph();
		expect(await change_reaches_server(graph, ENTRY, { type: 'update', id: OTHER })).toBe(false);
	});
});

describe('start_watched_server', () => {
	test('the initial adds restart nothing', async () => {
		const { mock, program } = create_spawn();
		const { graph } = create_graph();
		const server = await start_watched_server(to_options({ program, graph }));
		await wait(40);
		expect(mock.calls).toHaveLength(1);
		await server.stop();
	});

	test('a change in the closure restarts once, a burst coalescing', async () => {
		const { mock, children, program } = create_spawn();
		const g = create_graph();
		const log = create_mock_logger();
		const server = await start_watched_server(to_options({ program, graph: g.graph, log }));
		g.emit({ type: 'update', id: DEP });
		g.emit({ type: 'update', id: DEEP });
		g.emit({ type: 'update', id: ENTRY });
		await until(() => mock.calls.length === 2);
		// one line for the burst, naming a change of it relative to `cwd`
		expect((log.info as any).mock.calls).toHaveLength(1);
		expect((log.info as any).mock.calls[0][0]).toMatch(
			/^\[test\] restarting server for update src\/(dep|deep|server)\.ts$/
		);
		await wait(40);
		expect(mock.calls).toHaveLength(2);
		expect(mock.calls[1]!.args).toEqual([ENTRY]);
		expect(server.process.child).toBe(children[1]);
		await server.stop();
	});

	test('a change outside the closure restarts nothing', async () => {
		const { mock, program } = create_spawn();
		const g = create_graph();
		const server = await start_watched_server(to_options({ program, graph: g.graph }));
		g.emit({ type: 'update', id: OTHER });
		await wait(40);
		expect(mock.calls).toHaveLength(1);
		await server.stop();
	});

	test('an added file the entry imports restarts it', async () => {
		const { mock, program } = create_spawn();
		const g = create_graph();
		const server = await start_watched_server(to_options({ program, graph: g.graph }));
		g.importers['/p/src/new.ts'] = [ENTRY];
		g.emit({ type: 'add', id: '/p/src/new.ts' });
		await until(() => mock.calls.length === 2);
		await server.stop();
	});

	test('a server that exits on its own is logged and comes back on the next change', async () => {
		const { mock, children, program } = create_spawn();
		const g = create_graph();
		const log = create_mock_logger();
		const server = await start_watched_server(to_options({ program, graph: g.graph, log }));
		children[0]!.end({ kind: 'exited', code: 1 });
		await until(() => (log.error as any).mock.calls.length === 1);
		expect((log.error as any).mock.calls[0][0]).toBe(
			'[test] server exited with code 1, restarting on the next change'
		);
		await wait(40);
		expect(mock.calls).toHaveLength(1);
		g.emit({ type: 'update', id: DEP });
		await until(() => mock.calls.length === 2);
		await server.stop();
		// neither the restart's stop nor the final one is logged as an exit
		expect((log.error as any).mock.calls).toHaveLength(1);
	});

	test('a failed restart is logged and the next change retries', async () => {
		const { mock, program } = create_spawn();
		const g = create_graph();
		const log = create_mock_logger();
		const server = await start_watched_server(to_options({ program, graph: g.graph, log }));
		const errors = vi.mocked(log.error).mock.calls;
		const infos = vi.mocked(log.info).mock.calls;
		mock.start_error = () => new SpawnError('launch_failed', 'node', 'launch_failed: nope');
		g.emit({ type: 'update', id: DEP });
		await until(() => errors.length === 1);
		expect(server.process.child).toBe(null);
		// a second failure, after the first settled, is its own restart, logged again
		g.emit({ type: 'update', id: DEEP });
		await until(() => errors.length === 2);
		expect(server.process.child).toBe(null);
		expect(infos).toEqual([
			['[test] restarting server for update src/dep.ts'],
			['[test] restarting server for update src/deep.ts']
		]);
		mock.start_error = () => undefined;
		g.emit({ type: 'update', id: DEP });
		await until(() => server.process.child !== null);
		expect(errors).toHaveLength(2);
		expect(infos).toHaveLength(3);
		await server.stop();
	});

	test('a restart that fails to launch after stop logs nothing', async () => {
		const { mock, program } = create_spawn();
		slow_stops(mock);
		const g = create_graph();
		const log = create_mock_logger();
		const server = await start_watched_server(to_options({ program, graph: g.graph, log }));
		mock.start_error = () => new SpawnError('launch_failed', 'node', 'launch_failed: nope');
		g.emit({ type: 'update', id: DEP });
		// the restart is in flight once it's announced, its slow stop holding it there
		await until(() => vi.mocked(log.info).mock.calls.length === 1);
		await server.stop();
		// the stop queued behind the restart, so the restart's launch has failed by now
		expect(mock.calls).toHaveLength(2);
		expect(vi.mocked(log.error).mock.calls).toEqual([]);
	});

	test('stop unsubscribes, cancels a pending restart, and stops the server', async () => {
		const { mock, children, program } = create_spawn();
		const g = create_graph();
		const server = await start_watched_server(
			to_options({ program, graph: g.graph, restart_delay: 30 })
		);
		g.emit({ type: 'update', id: DEP });
		await wait(5);
		const stopping = server.stop();
		expect(server.stop()).toBe(stopping);
		await stopping;
		expect(g.unwatched).toBe(1);
		const result = await children[0]!.closed;
		expect(result).toEqual({ kind: 'signaled', signal: 'SIGTERM' });
		await wait(60);
		expect(mock.calls).toHaveLength(1);
		expect(server.process.active).toBe(false);
	});

	test('with no graph it starts once and logs an exit without promising a restart', async () => {
		const { children, program } = create_spawn();
		const log = create_mock_logger();
		const server = await start_watched_server(to_options({ program, log }));
		children[0]!.end({ kind: 'exited', code: 0 });
		await until(() => (log.error as any).mock.calls.length === 1);
		expect((log.error as any).mock.calls[0][0]).toBe('[test] server exited with code 0');
		await server.stop();
	});

	test('a launch failure unsubscribes and rejects', async () => {
		const { mock, program } = create_spawn();
		const g = create_graph();
		mock.start_error = () => new SpawnError('launch_failed', 'node', 'launch_failed: nope');
		const error = await start_watched_server(to_options({ program, graph: g.graph })).catch(
			(e: unknown) => e
		);
		assert(error instanceof SpawnError);
		expect(g.unwatched).toBe(1);
	});
});

describe('start_watched_server restarts sharing one in flight', () => {
	test('a crash after a shared restart is logged once', async () => {
		const { mock, children, program } = create_spawn();
		slow_stops(mock);
		const g = create_graph();
		const log = create_mock_logger();
		const server = await start_watched_server(to_options({ program, graph: g.graph, log }));
		g.emit({ type: 'update', id: DEP });
		await wait(30);
		g.emit({ type: 'update', id: DEP });
		await until(() => mock.calls.length === 2);
		await wait(40);
		expect(mock.calls).toHaveLength(2);
		children[1]!.end({ kind: 'exited', code: 1 });
		await until(() => (log.error as any).mock.calls.length === 1);
		await wait(20);
		expect((log.error as any).mock.calls).toHaveLength(1);
		await server.stop();
	});

	test('a shared restart that fails to launch is logged once', async () => {
		const { mock, program } = create_spawn();
		slow_stops(mock);
		const g = create_graph();
		const log = create_mock_logger();
		const server = await start_watched_server(to_options({ program, graph: g.graph, log }));
		mock.start_error = () => new SpawnError('launch_failed', 'node', 'launch_failed: nope');
		g.emit({ type: 'update', id: DEP });
		await wait(30);
		g.emit({ type: 'update', id: DEP });
		await until(() => (log.error as any).mock.calls.length >= 1);
		await wait(40);
		expect((log.error as any).mock.calls).toHaveLength(1);
		expect((log.error as any).mock.calls[0][0]).toBe(
			'[test] failed to restart server, retrying on the next change'
		);
		await server.stop();
	});

	test('a shared restart that succeeds logs one line, naming the change that began it', async () => {
		const { mock, program } = create_spawn();
		slow_stops(mock);
		const g = create_graph();
		const log = create_mock_logger();
		const server = await start_watched_server(to_options({ program, graph: g.graph, log }));
		g.emit({ type: 'update', id: DEP });
		await wait(30);
		g.emit({ type: 'update', id: DEEP });
		await until(() => mock.calls.length === 2);
		await wait(40);
		expect(mock.calls).toHaveLength(2);
		expect((log.info as any).mock.calls).toEqual([
			['[test] restarting server for update src/dep.ts']
		]);
		await server.stop();
	});
});
