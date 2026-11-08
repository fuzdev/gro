import type {
	Spawn,
	SpawnCallOptions,
	SpawnChild,
	SpawnProgram,
	SpawnProgramOptions,
	SpawnResult
} from '#lib/spawn.ts';
import { SpawnError } from '#lib/spawn_error.ts';

/**
 * A call a mock program received.
 */
export interface MockSpawnCall {
	method: 'run' | 'start';
	name: string;
	args: ReadonlyArray<string>;
	options: SpawnCallOptions | undefined;
	program_options: SpawnProgramOptions | undefined;
}

/**
 * A `Spawn` recording what it finds and runs. `available` says which names resolve,
 * by default every name in a project's `node_modules/.bin`, and `result` says how calls end.
 */
export interface MockSpawn extends Spawn {
	available: (name: string) => boolean;
	result: SpawnResult | ((call: MockSpawnCall) => SpawnResult);
	/**
	 * What `find` throws for a name, `undefined` to find it normally.
	 */
	find_error: (name: string) => Error | undefined;
	/**
	 * What `run` rejects with for a call, `undefined` to run normally.
	 */
	run_error: (call: MockSpawnCall) => Error | undefined;
	/**
	 * What `start` rejects with for a call, `undefined` to start normally.
	 */
	start_error: (call: MockSpawnCall) => Error | undefined;
	/**
	 * The child `start` resolves to, when `null` a closed child with no output.
	 */
	create_child: ((call: MockSpawnCall) => SpawnChild) | null;
	readonly finds: Array<string>;
	readonly calls: Array<MockSpawnCall>;
	/**
	 * Restores the defaults and forgets every find and call.
	 */
	reset: () => void;
}

const LOCAL_BIN = 'node_modules/.bin/';

const OK_RESULT: SpawnResult = { kind: 'exited', code: 0 };

const NO_ERROR = (): undefined => undefined;

/**
 * Creates a mock `SpawnChild`, closed with exit 0 and with no output unless overridden.
 */
export const create_mock_child = (overrides?: Partial<SpawnChild>): SpawnChild => ({
	pid: 1,
	closed: Promise.resolve(OK_RESULT),
	stdout: null,
	stderr: null,
	stdin: null,
	kill: () => true,
	[Symbol.asyncDispose]: async () => undefined,
	...overrides
});

/**
 * A child's output stream fed by hand, ending when the test says.
 */
export interface MockSpawnStream {
	stream: AsyncIterable<Uint8Array>;
	push: (chunk: Uint8Array) => void;
	end: () => void;
}

/**
 * Creates a `MockSpawnStream`.
 */
export const create_mock_stream = (): MockSpawnStream => {
	const queue: Array<Uint8Array> = [];
	let ended = false;
	let wake: (() => void) | null = null;
	const notify = (): void => {
		wake?.();
		wake = null;
	};
	return {
		stream: (async function* () {
			for (;;) {
				while (queue.length > 0) yield queue.shift()!;
				if (ended) return;
				await new Promise<void>((resolve) => {
					wake = resolve;
				});
			}
		})(),
		push: (chunk) => {
			queue.push(chunk);
			notify();
		},
		end: () => {
			ended = true;
			notify();
		}
	};
};

/**
 * Creates a mock `Spawn`, whose `narrow` and whose programs' `out` throw,
 * so code reaching an op the test didn't expect fails loudly.
 */
export const create_mock_spawn = (): MockSpawn => {
	const to_result = (call: MockSpawnCall, error: Error | undefined): SpawnResult => {
		mock.calls.push(call);
		if (error !== undefined) throw error;
		return typeof mock.result === 'function' ? mock.result(call) : mock.result;
	};
	const create_program = (
		name: string,
		program_options: SpawnProgramOptions | undefined
	): SpawnProgram => ({
		name,
		path: '/mock/' + name,
		run: async (args, options) => {
			const call: MockSpawnCall = { method: 'run', name, args, options, program_options };
			return to_result(call, mock.run_error(call));
		},
		out: () => {
			throw Error(`mock SpawnProgram.out not set: ${name}`);
		},
		start: async (args, options) => {
			const call: MockSpawnCall = { method: 'start', name, args, options, program_options };
			const result = to_result(call, mock.start_error(call));
			return mock.create_child?.(call) ?? create_mock_child({ closed: Promise.resolve(result) });
		}
	});
	const find = (name: string, options?: SpawnProgramOptions): SpawnProgram | null => {
		mock.finds.push(name);
		const error = mock.find_error(name);
		if (error !== undefined) throw error;
		return mock.available(name) ? create_program(name, options) : null;
	};
	const mock: MockSpawn = {
		available: (name) => name.startsWith(LOCAL_BIN),
		result: OK_RESULT,
		find_error: NO_ERROR,
		run_error: NO_ERROR,
		start_error: NO_ERROR,
		create_child: null,
		finds: [],
		calls: [],
		find,
		program: (name, options) => {
			const program = find(name, options);
			if (!program) throw new SpawnError('not_found', name, `not_found: no program ${name}`);
			return program;
		},
		narrow: () => {
			throw Error('mock Spawn.narrow not set');
		},
		reset: () => {
			mock.available = (name) => name.startsWith(LOCAL_BIN);
			mock.result = OK_RESULT;
			mock.find_error = NO_ERROR;
			mock.run_error = NO_ERROR;
			mock.start_error = NO_ERROR;
			mock.create_child = null;
			mock.finds.length = 0;
			mock.calls.length = 0;
		}
	};
	return mock;
};

/**
 * The spawn host module with `spawn` a mock, for a test of a task, the composition root that imports it:
 * `vi.mock('#lib/spawn.ts', async () => (await import('./spawn_mocks.ts')).create_mock_spawn_module())`.
 */
export const create_mock_spawn_module = (): typeof import('#lib/spawn.ts') => ({
	spawn: create_mock_spawn()
});

/**
 * The mocked `spawn`, in a test that mocks `#lib/spawn.ts` with `create_mock_spawn_module`.
 */
export const get_mock_spawn = async (): Promise<MockSpawn> =>
	(await import('#lib/spawn.ts')).spawn as unknown as MockSpawn;
