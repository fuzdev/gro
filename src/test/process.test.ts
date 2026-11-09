import { describe, test, assert, beforeEach, afterEach, vi } from 'vitest';
import { spawn as node_spawn, spawnSync } from 'node:child_process';
import { closeSync, existsSync, mkdirSync, openSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { spawn_with_loader_args } from '#lib/gro_helpers.ts';
import { paths } from '#lib/paths.ts';
import { process as process_host, type Env } from '#lib/process.ts';
import { ProcessError, is_process_error, type ProcessErrorKind } from '#lib/process_error.ts';
import { create_fs_sandbox, remove_fs_sandbox, type FsSandbox } from './fs_test_helpers.ts';
import { LOADER_SOURCE_PATH } from './test_helpers.ts';

/*

Every case that exits, reads a chosen environment, or breaks a pipe runs in a child through a
fixture script, under an environment of fake names only, so nothing here exits the test process,
signals outside a child, or reads the ambient environment.

*/

vi.setConfig({ testTimeout: 30_000 });

const MODULE_URL = new URL(join(paths.lib, 'process.ts'), 'file:').href;
const SPAWN_URL = new URL(join(paths.lib, 'spawn.ts'), 'file:').href;
const PROCESS_ERROR_URL = new URL(join(paths.lib, 'process_error.ts'), 'file:').href;

let sandbox: FsSandbox;

beforeEach(async () => {
	sandbox = await create_fs_sandbox();
});

afterEach(() => {
	remove_fs_sandbox(sandbox);
});

interface RunOptions {
	env?: Record<string, string>;
	input?: string;
}

const run_node = (
	args: Array<string>,
	options: RunOptions = {}
): { status: number | null; stdout: string; stderr: string } => {
	const { status, stdout, stderr } = spawnSync(process.execPath, args, {
		cwd: sandbox.root,
		env: options.env ?? {},
		input: options.input ?? '',
		encoding: 'utf8',
		timeout: 20_000
	});
	return { status, stdout, stderr };
};

const write_fixture = (body: string): string => {
	const path = join(sandbox.base, 'fixture.ts');
	writeFileSync(path, `import {process as p} from ${JSON.stringify(MODULE_URL)};\n${body}\n`);
	return path;
};

const run_fixture = (
	body: string,
	options: RunOptions = {},
	args: Array<string> = []
): { status: number | null; stdout: string; stderr: string } =>
	run_node([write_fixture(body), ...args], options);

const run_fixture_json = (
	body: string,
	options: RunOptions = {},
	args: Array<string> = []
): unknown => {
	const result = run_fixture(body, options, args);
	assert.strictEqual(result.status, 0, result.stderr);
	return JSON.parse(result.stdout);
};

const assert_process_error = (fn: () => unknown, kind: ProcessErrorKind): ProcessError => {
	try {
		fn();
	} catch (error) {
		assert.ok(is_process_error(error, kind), `expected ${kind}, got ${String(error)}`);
		return error;
	}
	throw new Error(`expected a ProcessError of kind ${kind}`);
};

const assert_gone = async (pid: number): Promise<void> => {
	for (let i = 0; i < 600; i++) {
		try {
			process.kill(pid, 0);
		} catch {
			return;
		}
		await new Promise((r) => setTimeout(r, 5));
	}
	assert.fail(`pid ${pid} is still running`);
};

describe('ProcessError', () => {
	test('is typed and frozen, and matches across copies of the class', () => {
		const error = new ProcessError('not_granted', 'A_NAME');
		assert.strictEqual(error.kind, 'not_granted');
		assert.strictEqual(error.target, 'A_NAME');
		assert.ok(Object.isFrozen(ProcessError));
		assert.ok(Object.isFrozen(ProcessError.prototype));
		assert.ok(is_process_error(error, 'not_granted'));
		assert.ok(!is_process_error(error, 'io_error'));
		class Other extends Error {
			override name = 'ProcessError';
			kind = 'io_error';
		}
		Object.defineProperty(Other, 'name', { value: 'ProcessError' });
		assert.ok(is_process_error(new Other(), 'io_error'));
		assert.ok(!is_process_error(new Error('x')));
	});
});

describe('env', () => {
	test('reads a variable by name, an unset one as undefined', () => {
		const result = run_fixture_json(
			`console.log(JSON.stringify([p.env.get('GRO_FAKE_A'), p.env.get('GRO_FAKE_EMPTY'), p.env.get('GRO_FAKE_UNSET') === undefined]));`,
			{ env: { GRO_FAKE_A: 'fake-a', GRO_FAKE_EMPTY: '' } }
		);
		assert.deepEqual(result, ['fake-a', '', true]);
	});

	test('`get_all` is a frozen null-prototype record of the variables', () => {
		const result = run_fixture_json(
			`const all = p.env.get_all();
			 console.log(JSON.stringify({
				proto: Object.getPrototypeOf(all) === null,
				frozen: Object.isFrozen(all),
				a: all.GRO_FAKE_A,
				keys: Object.keys(all),
			 }));`,
			{ env: { GRO_FAKE_A: 'fake-a' } }
		);
		assert.deepEqual(result, { proto: true, frozen: true, a: 'fake-a', keys: ['GRO_FAKE_A'] });
	});

	test('an inherited name is no variable, even with a polluted prototype', () => {
		const result = run_fixture_json(
			`Object.prototype.GRO_FAKE_POLLUTED = 'polluted';
			 const narrowed = p.env.narrow(['toString', 'GRO_FAKE_POLLUTED']);
			 console.log(JSON.stringify([
				typeof p.env.get('toString'),
				typeof p.env.get('hasOwnProperty'),
				typeof p.env.get('__proto__'),
				typeof p.env.get('GRO_FAKE_POLLUTED'),
				typeof narrowed.get('toString'),
				typeof narrowed.get('GRO_FAKE_POLLUTED'),
				'GRO_FAKE_POLLUTED' in p.env.get_all()
			 ]));`
		);
		assert.deepEqual(result, [
			'undefined',
			'undefined',
			'undefined',
			'undefined',
			'undefined',
			'undefined',
			false
		]);
	});

	test('the environment is a snapshot taken when the module loads', () => {
		const result = run_fixture_json(
			`globalThis.process.env.GRO_FAKE_LATE = 'late';
			 delete globalThis.process.env.GRO_FAKE_A;
			 console.log(JSON.stringify([p.env.get('GRO_FAKE_A'), p.env.get('GRO_FAKE_LATE') === undefined]));`,
			{ env: { GRO_FAKE_A: 'fake-a' } }
		);
		assert.deepEqual(result, ['fake-a', true]);
	});

	test('a write before the module loads is in the snapshot', () => {
		const path = join(sandbox.base, 'early.ts');
		writeFileSync(
			path,
			`globalThis.process.env.GRO_FAKE_EARLY = 'early';
			 const {process: p} = await import(${JSON.stringify(MODULE_URL)});
			 console.log(JSON.stringify(p.env.get('GRO_FAKE_EARLY')));`
		);
		const { status, stdout, stderr } = run_node([path]);
		assert.strictEqual(status, 0, stderr);
		assert.deepEqual(JSON.parse(stdout), 'early');
	});

	test('refuses a malformed name as a TypeError, not a missing variable', () => {
		for (const bad of ['', 'A=B', 'A\0B', '\ud800', 1, null, undefined, {}]) {
			assert.throws(() => process_host.env.get(bad as string), TypeError);
		}
	});

	test('a refused name with a `=` does not echo what could be a value', () => {
		for (const fn of [
			() => process_host.env.get('GRO_FAKE=fake-secret'),
			() => process_host.env.narrow(['GRO_FAKE=fake-secret'])
		]) {
			assert.throws(fn, TypeError);
			try {
				fn();
			} catch (error) {
				assert.notInclude((error as Error).message, 'fake-secret');
			}
		}
	});

	describe('narrow', () => {
		let narrowed: Env;

		beforeEach(() => {
			narrowed = process_host.env.narrow(['GRO_FAKE_A', 'GRO_FAKE_B']);
		});

		test('a name outside the set throws `not_granted`, it does not read as unset', () => {
			const error = assert_process_error(() => narrowed.get('GRO_FAKE_C'), 'not_granted');
			assert.strictEqual(error.target, 'GRO_FAKE_C');
			assert.notInclude(error.message, 'fake');
		});

		test('a name in the set that is unset reads as undefined', () => {
			assert.strictEqual(narrowed.get('GRO_FAKE_A'), undefined);
		});

		test('reads through only the set, from the parent snapshot', () => {
			const result = run_fixture_json(
				`const narrowed = p.env.narrow(['GRO_FAKE_A', 'GRO_FAKE_UNSET']);
				 let refused = false;
				 try { narrowed.get('GRO_FAKE_B'); } catch (e) { refused = e.kind === 'not_granted'; }
				 console.log(JSON.stringify({
					a: narrowed.get('GRO_FAKE_A'),
					unset: narrowed.get('GRO_FAKE_UNSET') === undefined,
					refused,
					all: Object.keys(narrowed.get_all()),
					null_proto: Object.getPrototypeOf(narrowed.get_all()) === null
				 }));`,
				{ env: { GRO_FAKE_A: 'fake-a', GRO_FAKE_B: 'fake-b' } }
			);
			assert.deepEqual(result, {
				a: 'fake-a',
				unset: true,
				refused: true,
				all: ['GRO_FAKE_A'],
				null_proto: true
			});
		});

		test('only shrinks, as a name outside the parent is `not_granted`', () => {
			narrowed.narrow(['GRO_FAKE_A']);
			assert_process_error(() => narrowed.narrow(['GRO_FAKE_C']), 'not_granted');
		});

		test('takes exact names only, a glob or `*` is a TypeError', () => {
			for (const bad of ['GRO_FAKE_*', '*', 'A*B']) {
				assert.throws(() => process_host.env.narrow([bad]), TypeError);
				assert.throws(() => narrowed.narrow([bad]), TypeError);
			}
		});

		test('refuses a non-array and malformed names', () => {
			assert.throws(
				() => process_host.env.narrow('GRO_FAKE_A' as unknown as Array<string>),
				TypeError
			);
			for (const bad of ['', 'A=B', 'A\0B', '\ud800', 1]) {
				assert.throws(() => process_host.env.narrow([bad as string]), TypeError);
			}
		});

		test('reads the array once, element by element', () => {
			let length_reads = 0;
			let index_reads = 0;
			const names = new Proxy(['GRO_FAKE_A'], {
				get(target, prop) {
					if (prop === 'length') length_reads++;
					if (prop === '0') index_reads++;
					return Reflect.get(target, prop);
				}
			});
			process_host.env.narrow(names);
			assert.strictEqual(length_reads, 1);
			assert.strictEqual(index_reads, 1);
		});
	});
});

describe('args', () => {
	const ARGS_FIXTURE = 'console.log(JSON.stringify(p.args));';

	test('are the arguments after the script under bare node, frozen', () => {
		const result = run_fixture_json(ARGS_FIXTURE, {}, ['a', '--b', '-c', '--', 'd']);
		assert.deepEqual(result, ['a', '--b', '-c', '--', 'd']);
		assert.ok(
			run_fixture_json('console.log(Object.isFrozen(p.args));') === true,
			'the array is frozen'
		);
	});

	test('are the arguments under the loader, as `gro run` starts the script', () => {
		const script = write_fixture(ARGS_FIXTURE);
		const { status, stdout, stderr } = run_node(
			spawn_with_loader_args(LOADER_SOURCE_PATH, script, ['--help', 'a', '--', 'b']),
			{ env: { PATH: '/usr/bin:/bin' } }
		);
		assert.strictEqual(status, 0, stderr);
		assert.deepEqual(JSON.parse(stdout.trim()), ['--help', 'a', '--', 'b']);
	});

	test('under the CLI they are the CLI args, the task name included', () => {
		const task = join(sandbox.root, 'args.task.ts');
		writeFileSync(
			task,
			`import {process as p} from ${JSON.stringify(MODULE_URL)};
			 export const task = {summary: 'x', run: () => { console.log('ARGS' + JSON.stringify(p.args)); }};`
		);
		const { status, stdout, stderr } = run_node(
			spawn_with_loader_args(LOADER_SOURCE_PATH, join(paths.lib, 'invoke.ts'), [
				'task',
				'args.task.ts',
				'--flag',
				'v',
				'--',
				'other'
			]),
			{ env: { PATH: '/usr/bin:/bin' } }
		);
		assert.strictEqual(status, 0, stderr);
		const line = stdout.split('\n').find((l) => l.startsWith('ARGS'))!;
		assert.deepEqual(JSON.parse(line.slice(4)), [
			'task',
			'args.task.ts',
			'--flag',
			'v',
			'--',
			'other'
		]);
	});

	test('`node -e` has no script, so the first argument is the first', () => {
		const result = run_node([
			'-e',
			`import(${JSON.stringify(MODULE_URL)}).then((m) => console.log(JSON.stringify(m.process.args)))`,
			'a',
			'b'
		]);
		assert.strictEqual(result.status, 0, result.stderr);
		assert.deepEqual(JSON.parse(result.stdout), ['a', 'b']);
	});

	test('a script read from stdin as `-` keeps its arguments', () => {
		const result = run_node(['-', 'a', 'b'], {
			input: `import(${JSON.stringify(MODULE_URL)}).then((m) => console.log(JSON.stringify(m.process.args)));`
		});
		assert.strictEqual(result.status, 0, result.stderr);
		assert.deepEqual(JSON.parse(result.stdout), ['a', 'b']);
	});

	const WORKER_POST_ARGS = `import(${JSON.stringify(MODULE_URL)}).then((m) => require('node:worker_threads').parentPort.postMessage(m.process.args));`;

	test('an eval worker has its own arguments, without the eval marker', () => {
		const result = run_fixture_json(
			`import {Worker} from 'node:worker_threads';
			 const worker = new Worker(${JSON.stringify(WORKER_POST_ARGS)}, {eval: true, argv: ['a', 'b']});
			 worker.on('message', (args) => console.log(JSON.stringify(args)));`
		);
		assert.deepEqual(result, ['a', 'b']);
	});

	test('a file worker started from a `node -e` parent has its own arguments', () => {
		const worker_path = join(sandbox.base, 'worker.cjs');
		writeFileSync(worker_path, WORKER_POST_ARGS);
		const result = run_node([
			'-e',
			`const {Worker} = require('node:worker_threads');
			 const worker = new Worker(${JSON.stringify(worker_path)}, {argv: ['a', 'b']});
			 worker.on('message', (args) => console.log(JSON.stringify(args)));`
		]);
		assert.strictEqual(result.status, 0, result.stderr);
		assert.deepEqual(JSON.parse(result.stdout), ['a', 'b']);
	});

	test('are a frozen real array, so unlike the handles awaiting them adopts a polluted `then`', () => {
		const result = run_fixture_json(
			`Object.prototype.then = function (ok) { delete Object.prototype.then; ok('forged'); };
			 const awaited = await p.args;
			 console.log(JSON.stringify({
				awaited,
				is_array: Array.isArray(p.args),
				frozen: Object.isFrozen(p.args),
				proto: Object.getPrototypeOf(p.args) === Array.prototype,
			 }));`
		);
		assert.deepEqual(result, { awaited: 'forged', is_array: true, frozen: true, proto: true });
	});
});

describe('cwd and identity', () => {
	test('`cwd` is the checked working directory', () => {
		assert.deepEqual(run_fixture_json('console.log(JSON.stringify(p.cwd()));'), sandbox.root);
	});

	test('`pid` and `runtime_version` are the process and the runtime', () => {
		assert.strictEqual(process_host.pid, process.pid);
		assert.strictEqual(process_host.runtime_version, `node/${process.version}`);
	});

	test('`is_terminal` reports a pipe as no terminal', () => {
		assert.deepEqual(
			run_fixture_json(
				'console.log(JSON.stringify([p.stdin.is_terminal, p.stdout.is_terminal, p.stderr.is_terminal]));'
			),
			[false, false, false]
		);
	});
});

describe('exit', () => {
	const CODE_CHECKS = `
		const bad = [256, -1, 1.5, NaN, Infinity, '1', null, true, {}, [], 1n, Symbol('s')];
		let called = false;
		bad.push({ then(ok) { called = true; ok(1); } });
		bad.push({ [Symbol.toPrimitive]() { called = true; return 1; } });
		let refused = 0;
		for (const code of bad) {
			try { p.exit(code); } catch (e) { if (e instanceof TypeError) refused++; }
			try { p.set_exit_code(code); } catch (e) { if (e instanceof TypeError) refused++; }
		}
		console.log(JSON.stringify({ refused, total: bad.length * 2, called }));
	`;

	test('exits with the code', () => {
		assert.strictEqual(run_fixture('p.exit(5);').status, 5);
		assert.strictEqual(run_fixture('p.exit(0);').status, 0);
		assert.strictEqual(run_fixture('p.exit(255);').status, 255);
	});

	test('with no code uses `set_exit_code`, else 0', () => {
		assert.strictEqual(run_fixture('p.set_exit_code(7); p.exit();').status, 7);
		assert.strictEqual(run_fixture('p.exit();').status, 0);
	});

	test('`set_exit_code` applies when the process ends on its own', () => {
		assert.strictEqual(run_fixture('p.set_exit_code(9);').status, 9);
	});

	test('refuses every other code with a TypeError, running none of the code it was given', () => {
		const result = run_fixture_json(CODE_CHECKS) as Record<string, unknown>;
		assert.strictEqual(result.refused, result.total);
		assert.strictEqual(result.called, false);
	});

	test('runs the spawn module`s child cleanup', async () => {
		const bin = join(sandbox.base, 'bin');
		mkdirSync(bin);
		const ready = join(sandbox.base, 'pid');
		const sleeper = join(bin, 'sleeper');
		writeFileSync(sleeper, `#!/bin/sh\necho $$ > '${ready}'\nexec sleep 100\n`, { mode: 0o755 });
		const result = run_fixture(
			`import {spawn} from ${JSON.stringify(SPAWN_URL)};
			 import {existsSync} from 'node:fs';
			 await spawn.program('sleeper').start([], {stdio: 'pipe'});
			 for (let i = 0; i < 2000 && !existsSync(${JSON.stringify(ready)}); i++) {
				await new Promise((r) => setTimeout(r, 5));
			 }
			 p.exit(3);`,
			{ env: { PATH: `${bin}:/usr/bin:/bin` } }
		);
		assert.strictEqual(result.status, 3, result.stderr);
		assert.ok(existsSync(ready));
		await assert_gone(Number(readFileSync(ready, 'utf8').trim()));
	});
});

describe('stdout and stderr', () => {
	test('write strings and bytes, resolving true', () => {
		const { status, stdout, stderr } = run_fixture(
			`const results = [];
			 results.push(await p.stdout.write('héllo '));
			 results.push(await p.stdout.write(new Uint8Array([119, 111, 114, 108, 100])));
			 results.push(await p.stderr.write('err'));
			 await p.stderr.write(JSON.stringify(results));`
		);
		assert.strictEqual(status, 0, stderr);
		assert.strictEqual(stdout, 'héllo world');
		assert.strictEqual(stderr, 'err[true,true,true]');
	});

	test('refuses data that is neither a well-formed string nor bytes', () => {
		const result = run_fixture_json(
			`let refused = 0;
			 for (const bad of ['\\ud800', 1, null, {}, new ArrayBuffer(1)]) {
				try { await p.stdout.write(bad); } catch (e) { if (e instanceof TypeError) refused++; }
			 }
			 console.log(JSON.stringify(refused));`
		);
		assert.strictEqual(result, 5);
	});

	test('an empty write resolves true', () => {
		const result = run_fixture_json(
			`console.log(JSON.stringify([await p.stdout.write(''), await p.stdout.write(new Uint8Array(0))]));`
		);
		assert.deepEqual(result, [true, true]);
	});

	describe('a failing write leaves other writers alone', () => {
		// stdout is `/dev/full`, so every write fails with `ENOSPC` and nothing reads it
		const run_on_full = (body: string): { status: number | null; stderr: string } => {
			const fd = openSync('/dev/full', 'w');
			try {
				const { status, stderr } = spawnSync(process.execPath, [write_fixture(body)], {
					cwd: sandbox.root,
					env: {},
					stdio: ['ignore', fd, 'pipe'],
					encoding: 'utf8',
					timeout: 20_000
				});
				return { status, stderr };
			} finally {
				closeSync(fd);
			}
		};

		const WRITE_FAILS = `
			const kind = await p.stdout.write('x').catch((e) => e.kind);
			await new Promise((r) => setImmediate(r));
			const { stdout } = globalThis.process;
			await p.stderr.write(JSON.stringify({ kind, listeners: stdout.listenerCount('error') }) + '\\n');
		`;

		test('a later bare write still throws, with no listener left behind', () => {
			const { status, stderr } = run_on_full(`${WRITE_FAILS} stdout.write('bare');`);
			assert.notStrictEqual(status, 0);
			assert.deepEqual(JSON.parse(stderr.split('\n')[0]!), { kind: 'io_error', listeners: 0 });
			assert.include(stderr, 'ENOSPC');
		});

		test('a listener of another writer still gets the error', () => {
			const { status, stderr } = run_on_full(
				`let seen = 0;
				 globalThis.process.stdout.on('error', () => { seen++; });
				 ${WRITE_FAILS}
				 stdout.write('bare');
				 await new Promise((r) => setImmediate(r));
				 await p.stderr.write(JSON.stringify(seen));`
			);
			assert.strictEqual(status, 0, stderr);
			assert.deepEqual(stderr.split('\n'), [
				JSON.stringify({ kind: 'io_error', listeners: 1 }),
				'2'
			]);
		});
	});

	test('a broken pipe resolves false, with no crash or stack', async () => {
		const child = node_spawn(
			process.execPath,
			[
				write_fixture(
					`let last = null;
					 for (let i = 0; i < 2000; i++) {
						last = await p.stdout.write('x'.repeat(100000));
						if (last === false) break;
					 }
					 const after = await p.stdout.write('more');
					 await p.stderr.write(JSON.stringify({ last, after }));`
				)
			],
			{ cwd: sandbox.root, env: {}, stdio: ['ignore', 'pipe', 'pipe'] }
		);
		const stderr: Array<Buffer> = [];
		child.stderr.on('data', (chunk: Buffer) => stderr.push(chunk));
		await new Promise<void>((resolve) => {
			child.stdout.once('data', () => {
				child.stdout.destroy();
				resolve();
			});
		});
		const code = await new Promise<number | null>((resolve) => {
			child.on('close', (c) => resolve(c));
		});
		const text = Buffer.concat(stderr).toString();
		assert.strictEqual(code, 0, text);
		assert.notInclude(text, 'Error');
		assert.notInclude(text, 'at ');
		assert.deepEqual(JSON.parse(text), { last: false, after: false });
	});
});

describe('stdin', () => {
	test('reads bytes as plain `Uint8Array` copies, through to the end', () => {
		const result = run_fixture_json(
			`let n = 0;
			 let plain = true;
			 for await (const chunk of p.stdin) {
				n += chunk.length;
				if (!(chunk instanceof Uint8Array) || Buffer.isBuffer(chunk)) plain = false;
			 }
			 console.log(JSON.stringify({ n, plain }));`,
			{ input: 'hello' }
		);
		assert.deepEqual(result, { n: 5, plain: true });
	});

	test('reads once: a second iteration is a TypeError', () => {
		const result = run_fixture_json(
			`p.stdin[Symbol.asyncIterator]();
			 let second = false;
			 try { p.stdin[Symbol.asyncIterator](); } catch (e) { second = e instanceof TypeError; }
			 console.log(JSON.stringify(second));`
		);
		assert.strictEqual(result, true);
	});

	test('leaving a loop early lets the process exit with stdin still open', async () => {
		const child = node_spawn(
			process.execPath,
			[write_fixture(`for await (const chunk of p.stdin) { await p.stdout.write('got'); break; }`)],
			{ cwd: sandbox.root, env: {}, stdio: ['pipe', 'pipe', 'pipe'] }
		);
		child.stdin.write('x');
		let stdout = '';
		child.stdout.on('data', (chunk: Buffer) => {
			stdout += chunk.toString();
		});
		const code = await new Promise<number | null>((resolve) => {
			child.on('close', (c) => resolve(c));
		});
		child.stdin.destroy();
		assert.strictEqual(code, 0);
		assert.strictEqual(stdout, 'got');
	});

	test('leaving a loop early leaves the rest to a child inheriting stdin', async () => {
		const LINES = Array.from({ length: 30 }, (_, i) => `line${i}\n`);
		const child = node_spawn(
			process.execPath,
			[
				write_fixture(
					`import {spawn} from 'node:child_process';
					 for await (const chunk of p.stdin) break;
					 const inheritor = spawn(globalThis.process.execPath, ['-e', 'process.stdin.pipe(process.stdout)'], {
						stdio: ['inherit', 'inherit', 'inherit'],
					 });
					 inheritor.on('spawn', () => { globalThis.process.stdout.write('ready'); });
					 await new Promise((resolve) => inheritor.on('close', resolve));`
				)
			],
			{ cwd: sandbox.root, env: {}, stdio: ['pipe', 'pipe', 'inherit'] }
		);
		let stdout = '';
		const ready = new Promise<void>((resolve) => {
			child.stdout.on('data', (chunk: Buffer) => {
				stdout += chunk.toString();
				if (stdout.startsWith('ready')) resolve();
			});
		});
		child.stdin.write('first\n');
		await ready;
		// spaced out, so a read still open in the parent races each line
		for (const line of LINES) {
			child.stdin.write(line);
			await new Promise((r) => setTimeout(r, 5));
		}
		child.stdin.end();
		const code = await new Promise<number | null>((resolve) => {
			child.on('close', (c) => resolve(c));
		});
		assert.strictEqual(code, 0);
		assert.strictEqual(stdout, `ready${LINES.join('')}`);
	});

	test('a chunk that is not bytes is an `io_error`, whether from an encoding or a forged result', () => {
		const encoded = run_fixture_json(
			`globalThis.process.stdin.setEncoding('utf8');
			 let kind = null;
			 try { for await (const chunk of p.stdin) void chunk; } catch (e) { kind = e.kind; }
			 console.log(JSON.stringify(kind));`,
			{ input: 'hello\n' }
		);
		assert.strictEqual(encoded, 'io_error');
		const forged = run_fixture_json(
			`const iterator = p.stdin[Symbol.asyncIterator]();
			 Object.prototype.then = function (ok) { delete Object.prototype.then; ok({ done: false, value: 'forged' }); };
			 let kind = null;
			 try { await iterator.next(); } catch (e) { kind = e.kind; }
			 delete Object.prototype.then;
			 console.log(JSON.stringify(kind));`,
			{ input: 'hello\n' }
		);
		assert.strictEqual(forged, 'io_error');
	});
});

describe('handle hardening', () => {
	// run in a child, as `exit` and `set_exit_code` act on the process that calls them
	const check_handles = (kind: 'env' | 'streams' | 'process'): unknown =>
		run_fixture_json(
			`import {ProcessError} from ${JSON.stringify(PROCESS_ERROR_URL)};
			 const fakes = () => [{}, Object.create(null), { pid: 1 }, null, undefined, globalThis.process];
			 const handles = {
				env: { root: p.env, narrowed: p.env.narrow(['GRO_FAKE_A']) },
				streams: { stdin: p.stdin, stdout: p.stdout, stderr: p.stderr, input_iterator: p.stdin[Symbol.asyncIterator]() },
				process: { process: p },
			 }[${JSON.stringify(kind)}];
			 const seen = [];
			 const failures = [];
			 for (const [name, handle] of Object.entries(handles)) {
				const proto = Object.getPrototypeOf(handle);
				for (const key of Reflect.ownKeys(proto)) {
					if (key === 'constructor') continue;
					seen.push(name + '.' + String(key));
					const desc = Object.getOwnPropertyDescriptor(proto, key);
					const fn = desc.value ?? desc.get;
					// args that pass the method's own checks, so only the receiver can throw
					const args = { exit: [3], set_exit_code: [3], narrow: [['GRO_FAKE_A']], get: ['GRO_FAKE_A'] }[String(key)] ?? [];
					for (const fake of fakes()) {
						try {
							const r = fn.apply(fake, args);
							if (r && typeof r.then === 'function') await r;
							failures.push(name + '.' + String(key) + ' accepted a fake receiver');
						} catch (e) {
							if (!(e instanceof TypeError)) failures.push(name + '.' + String(key) + ' threw ' + e);
						}
					}
				}
				const Ctor = proto.constructor;
				if (Ctor) {
					let refused = false;
					try { new Ctor(Symbol('x'), {}, null); } catch (e) { refused = e instanceof TypeError; }
					if (!refused) failures.push(name + ' constructor accepted a forged key');
				}
				if (!Object.isFrozen(handle) || !Object.isFrozen(proto) || Object.getPrototypeOf(proto) !== null) {
					failures.push(name + ' is not frozen and null-prototype');
				}
			 }
			 console.log(JSON.stringify({ seen, failures }));`
		);

	test('no method or getter acts on a borrowed receiver', () => {
		const expected: Record<string, Array<string>> = {
			env: [
				'root.get',
				'root.get_all',
				'root.narrow',
				'narrowed.get',
				'narrowed.get_all',
				'narrowed.narrow'
			],
			streams: [
				'stdin.is_terminal',
				'stdin.Symbol(Symbol.asyncIterator)',
				'stdout.is_terminal',
				'stdout.write',
				'stderr.is_terminal',
				'stderr.write',
				'input_iterator.next',
				'input_iterator.return'
			],
			process: ['process.cwd', 'process.exit', 'process.set_exit_code']
		};
		for (const kind of ['env', 'streams', 'process'] as const) {
			const { seen, failures } = check_handles(kind) as {
				seen: Array<string>;
				failures: Array<string>;
			};
			assert.deepEqual(failures, [], kind);
			for (const name of expected[kind]!) assert.include(seen, name, kind);
		}
	});

	test('`process` and its parts are frozen, with no thenable surface', () => {
		const result = run_fixture_json(
			`Object.prototype.then = function (ok) { ok('forged'); };
			 const results = [];
			 for (const handle of [p, p.env, p.env.narrow(['A']), p.stdin, p.stdout, p.env.get_all()]) {
				results.push(await handle);
			 }
			 delete Object.prototype.then;
			 const frozen = [p, p.env, p.args, p.stdin, p.stdout, p.stderr, p.env.get_all()].every(Object.isFrozen);
			 const assigned = (() => { try { p.pid = 1; return true; } catch { return false; } })();
			 console.log(JSON.stringify({ forged: results.includes('forged'), frozen, assigned, pid: p.pid === globalThis.process.pid }));`
		);
		assert.deepEqual(result, { forged: false, frozen: true, assigned: false, pid: true });
	});

	test('`process.constructor` refuses without the key', () => {
		const result = run_fixture_json(
			`let refused = 0;
			 for (const handle of [p, p.env, p.stdin, p.stdout]) {
				try { new handle.constructor(Symbol('forged')); } catch (e) { if (e instanceof TypeError) refused++; }
			 }
			 console.log(JSON.stringify(refused));`
		);
		assert.strictEqual(result, 4);
	});
});
