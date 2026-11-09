import { describe, test, expect } from 'vitest';

import { spawn_restartable } from '#lib/spawn_restartable.ts';
import type { SpawnChild, SpawnResult } from '#lib/spawn.ts';
import { SpawnError } from '#lib/spawn_error.ts';

import { create_mock_child, create_mock_spawn, type MockSpawn } from './spawn_mocks.ts';

/**
 * A mock spawn whose children record their disposal, `closed` resolving once disposed.
 */
const create_spawn = (): MockSpawn & { children: Array<SpawnChild>; disposed: Array<number> } => {
	const mock = create_mock_spawn();
	mock.available = () => true;
	const children: Array<SpawnChild> = [];
	const disposed: Array<number> = [];
	mock.create_child = () => {
		const id = children.length;
		let resolve_closed!: (result: SpawnResult) => void;
		const closed = new Promise<SpawnResult>((resolve) => {
			resolve_closed = resolve;
		});
		const child = create_mock_child({
			pid: id,
			closed,
			[Symbol.asyncDispose]: async () => {
				disposed.push(id);
				resolve_closed({ kind: 'signaled', signal: 'SIGTERM' });
				await closed;
			}
		});
		children.push(child);
		return child;
	};
	return Object.assign(mock, { children, disposed });
};

describe('spawn_restartable', () => {
	test('resolves once the first child is running', async () => {
		const mock = create_spawn();
		const restartable = await spawn_restartable(mock.program('deno'), ['run', 'server.ts']);
		expect(restartable.active).toBe(true);
		expect(restartable.child).toBe(mock.children[0]);
		expect(mock.calls).toHaveLength(1);
		expect(mock.calls[0]).toMatchObject({ method: 'start', args: ['run', 'server.ts'] });
	});

	test('restart stops the running child and starts the same program with the same args', async () => {
		const mock = create_spawn();
		const restartable = await spawn_restartable(mock.program('deno'), ['run'], {
			stdio: 'ignore'
		});
		await restartable.restart();
		expect(mock.disposed).toEqual([0]);
		expect(mock.calls).toHaveLength(2);
		expect(mock.calls[1]).toMatchObject({ name: 'deno', args: ['run'] });
		expect(mock.calls[1]!.options).toEqual({ stdio: 'ignore' });
		// one `program` call, the handle kept, so a restart reuses the pin and its env
		expect(mock.finds).toEqual(['deno']);
		expect(restartable.child).toBe(mock.children[1]);
	});

	test('concurrent restarts share one restart', async () => {
		const mock = create_spawn();
		const restartable = await spawn_restartable(mock.program('deno'));
		const restarts = [restartable.restart(), restartable.restart()];
		await Promise.all(restarts);
		expect(restarts[0]).toBe(restarts[1]);
		expect(mock.calls).toHaveLength(2);
		expect(mock.disposed).toEqual([0]);
	});

	test('stop disposes the child and waits for it, then is inactive', async () => {
		const mock = create_spawn();
		const restartable = await spawn_restartable(mock.program('deno'));
		await restartable.stop();
		expect(mock.disposed).toEqual([0]);
		expect(restartable.active).toBe(false);
		expect(restartable.child).toBeNull();
		await expect(restartable.closed).resolves.toEqual({ kind: 'signaled', signal: 'SIGTERM' });
	});

	test('a loop reading `closed` and `active` does not restart a stopped child', async () => {
		const mock = create_spawn();
		const restartable = await spawn_restartable(mock.program('deno'));
		let restarts = 0;
		const loop = (async () => {
			while (restartable.active) {
				await restartable.closed;
				if (!restartable.active) break;
				restarts++;
				await restartable.restart();
			}
		})();
		await restartable.stop();
		await loop;
		expect(restarts).toBe(0);
		expect(restartable.active).toBe(false);
		expect(restartable.child).toBeNull();
	});

	test('`active` follows the last request, for a loop racing a queued `stop` or `restart`', async () => {
		for (const [first, second, expected] of [
			['stop', 'restart', true],
			['restart', 'stop', false]
		] as const) {
			const mock = create_spawn();
			const restartable = await spawn_restartable(mock.program('deno'));
			const first_done = restartable[first]();
			const second_done = restartable[second]();
			expect(restartable.active).toBe(expected);
			await Promise.all([first_done, second_done]);
			expect(restartable.active).toBe(expected);
			await restartable.stop();
		}
	});

	test('stop during a restart stops the child that restart started', async () => {
		const mock = create_spawn();
		const restartable = await spawn_restartable(mock.program('deno'));
		const restarting = restartable.restart();
		await restartable.stop();
		await restarting;
		expect(mock.disposed).toEqual([0, 1]);
		expect(restartable.active).toBe(false);
	});

	test('stop, restart, stop ends with no running child', async () => {
		const mock = create_spawn();
		const restartable = await spawn_restartable(mock.program('deno'));
		const stops = [restartable.stop()];
		const restarting = restartable.restart();
		stops.push(restartable.stop());
		await Promise.all([...stops, restarting]);
		expect(mock.calls).toHaveLength(2);
		expect(mock.disposed).toEqual([0, 1]);
		expect(restartable.child).toBeNull();
		expect(restartable.active).toBe(false);
	});

	test('restart, stop, restart ends with a running child', async () => {
		const mock = create_spawn();
		const restartable = await spawn_restartable(mock.program('deno'));
		const operations = [restartable.restart(), restartable.stop(), restartable.restart()];
		await Promise.all(operations);
		expect(mock.calls).toHaveLength(3);
		expect(mock.disposed).toEqual([0, 1]);
		expect(restartable.child).toBe(mock.children[2]);
		expect(restartable.active).toBe(true);
	});

	test('a stop settling leaves a restart queued behind it pending', async () => {
		const mock = create_spawn();
		const restartable = await spawn_restartable(mock.program('deno'));
		const first = restartable.stop();
		const restarting = restartable.restart();
		await first;
		// the restart is still queued, so this stop must wait for it and stop what it starts
		const second = restartable.stop();
		await Promise.all([restarting, second]);
		expect(mock.disposed).toEqual([0, 1]);
		expect(restartable.child).toBeNull();
		expect(restartable.active).toBe(false);
	});

	test('calls of one kind share the last pending operation, repeated stops included', async () => {
		const mock = create_spawn();
		const restartable = await spawn_restartable(mock.program('deno'));
		const first = restartable.stop();
		expect(restartable.stop()).toBe(first);
		const restarting = restartable.restart();
		const third = restartable.stop();
		expect(third).not.toBe(first);
		expect(restartable.stop()).toBe(third);
		await Promise.all([first, restarting, third]);
		expect(mock.disposed).toEqual([0, 1]);
		expect(restartable.active).toBe(false);
	});

	test("after a failed restart, `closed` is the previous child's and `child` is null", async () => {
		const mock = create_spawn();
		const restartable = await spawn_restartable(mock.program('deno'));
		const error = new SpawnError('launch_failed', 'deno', 'launch_failed: deno');
		mock.start_error = () => error;
		await expect(restartable.restart()).rejects.toBe(error);
		expect(restartable.child).toBeNull();
		expect(restartable.active).toBe(true);
		await expect(restartable.closed).resolves.toEqual({ kind: 'signaled', signal: 'SIGTERM' });
		// the queue survives the failure, so a later operation still runs
		mock.start_error = () => undefined;
		await restartable.restart();
		expect(restartable.child).toBe(mock.children[1]);
	});

	test('a restart after stop starts a child again', async () => {
		const mock = create_spawn();
		const restartable = await spawn_restartable(mock.program('deno'));
		await restartable.stop();
		await restartable.restart();
		expect(restartable.active).toBe(true);
		expect(restartable.child).toBe(mock.children[1]);
	});

	test('a failed start rejects, the first call and a restart alike', async () => {
		const mock = create_spawn();
		const error = new SpawnError('launch_failed', 'deno', 'launch_failed: deno');
		mock.start_error = () => error;
		await expect(spawn_restartable(mock.program('deno'))).rejects.toBe(error);

		mock.start_error = () => undefined;
		const restartable = await spawn_restartable(mock.program('deno'));
		mock.start_error = () => error;
		await expect(restartable.restart()).rejects.toBe(error);
	});
});
