import { describe, test, assert, vi } from 'vitest';
import { spawn as node_spawn, spawnSync } from 'node:child_process';
import { closeSync, openSync } from 'node:fs';

import { to_check_handles_body, to_lib_url, use_fixture_runner } from './host_fixture_helpers.ts';

vi.setConfig({ testTimeout: 30_000 });

const runner = use_fixture_runner(
	`import {stdout} from ${JSON.stringify(to_lib_url('stdout.ts'))};
	 import {stderr} from ${JSON.stringify(to_lib_url('stderr.ts'))};`
);
const { write_fixture, run_fixture, run_fixture_json } = runner;

describe('stdout and stderr', () => {
	test('write strings and bytes, resolving true', () => {
		const result = run_fixture(
			`const results = [];
			 results.push(await stdout.write('héllo '));
			 results.push(await stdout.write(new Uint8Array([119, 111, 114, 108, 100])));
			 results.push(await stderr.write('err'));
			 await stderr.write(JSON.stringify(results));`
		);
		assert.strictEqual(result.status, 0, result.stderr);
		assert.strictEqual(result.stdout, 'héllo world');
		assert.strictEqual(result.stderr, 'err[true,true,true]');
	});

	test('refuses data that is neither a well-formed string nor bytes', () => {
		const result = run_fixture_json(
			`let refused = 0;
			 for (const bad of ['\\ud800', 1, null, {}, new ArrayBuffer(1)]) {
				try { await stdout.write(bad); } catch (e) { if (e instanceof TypeError) refused++; }
			 }
			 console.log(JSON.stringify(refused));`
		);
		assert.strictEqual(result, 5);
	});

	test('an empty write resolves true', () => {
		const result = run_fixture_json(
			`console.log(JSON.stringify([await stdout.write(''), await stdout.write(new Uint8Array(0))]));`
		);
		assert.deepEqual(result, [true, true]);
	});

	describe('a failing write leaves other writers alone', () => {
		// stdout is `/dev/full`, so every write fails with `ENOSPC` and nothing reads it
		const run_on_full = (body: string): { status: number | null; stderr: string } => {
			const fd = openSync('/dev/full', 'w');
			try {
				const { status, stderr } = spawnSync(process.execPath, [write_fixture(body)], {
					cwd: runner.sandbox.root,
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
			const kind = await stdout.write('x').catch((e) => e.name + ' ' + e.kind);
			await new Promise((r) => setImmediate(r));
			const host_stdout = globalThis.process.stdout;
			await stderr.write(JSON.stringify({ kind, listeners: host_stdout.listenerCount('error') }) + '\\n');
		`;

		test('a later bare write still throws, with no listener left behind', () => {
			const { status, stderr } = run_on_full(`${WRITE_FAILS} host_stdout.write('bare');`);
			assert.notStrictEqual(status, 0);
			assert.deepEqual(JSON.parse(stderr.split('\n')[0]!), {
				kind: 'StdioError io_error',
				listeners: 0
			});
			assert.include(stderr, 'ENOSPC');
		});

		test('a listener of another writer still gets the error', () => {
			const { status, stderr } = run_on_full(
				`let seen = 0;
				 globalThis.process.stdout.on('error', () => { seen++; });
				 ${WRITE_FAILS}
				 host_stdout.write('bare');
				 await new Promise((r) => setImmediate(r));
				 await stderr.write(JSON.stringify(seen));`
			);
			assert.strictEqual(status, 0, stderr);
			assert.deepEqual(stderr.split('\n'), [
				JSON.stringify({ kind: 'StdioError io_error', listeners: 1 }),
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
						last = await stdout.write('x'.repeat(100000));
						if (last === false) break;
					 }
					 const after = await stdout.write('more');
					 await stderr.write(JSON.stringify({ last, after }));`
				)
			],
			{ cwd: runner.sandbox.root, env: {}, stdio: ['ignore', 'pipe', 'pipe'] }
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

describe('stdout', () => {
	test('`is_terminal` reports a pipe as no terminal', () => {
		assert.strictEqual(run_fixture_json('console.log(JSON.stringify(stdout.is_terminal));'), false);
	});

	test('no method or getter acts on a borrowed receiver', () => {
		const { seen, failures } = run_fixture_json(to_check_handles_body('{ stdout }')) as {
			seen: Array<string>;
			failures: Array<string>;
		};
		assert.deepEqual(failures, []);
		for (const name of ['stdout.is_terminal', 'stdout.write']) assert.include(seen, name);
	});

	test('is frozen, with no thenable surface', () => {
		const result = run_fixture_json(
			`Object.prototype.then = function (ok) { ok('forged'); };
			 const awaited = await stdout;
			 delete Object.prototype.then;
			 console.log(JSON.stringify({ forged: awaited === 'forged', frozen: Object.isFrozen(stdout) }));`
		);
		assert.deepEqual(result, { forged: false, frozen: true });
	});

	test('`stdout.constructor` refuses without the key', () => {
		const result = run_fixture_json(
			`let refused = false;
			 try { new stdout.constructor(Symbol('forged')); } catch (e) { refused = e instanceof TypeError; }
			 console.log(JSON.stringify(refused));`
		);
		assert.strictEqual(result, true);
	});
});
