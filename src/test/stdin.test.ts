import { describe, test, assert, vi } from 'vitest';
import { spawn as node_spawn } from 'node:child_process';

import { to_check_handles_body, to_lib_url, use_fixture_runner } from './host_fixture_helpers.ts';

vi.setConfig({ testTimeout: 30_000 });

const runner = use_fixture_runner(
	`import {stdin} from ${JSON.stringify(to_lib_url('stdin.ts'))};
	 import {stdout} from ${JSON.stringify(to_lib_url('stdout.ts'))};`
);
const { write_fixture, run_fixture_json } = runner;

describe('stdin', () => {
	test('reads bytes as plain `Uint8Array` copies, through to the end', () => {
		const result = run_fixture_json(
			`let n = 0;
			 let plain = true;
			 for await (const chunk of stdin) {
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
			`stdin[Symbol.asyncIterator]();
			 let second = false;
			 try { stdin[Symbol.asyncIterator](); } catch (e) { second = e instanceof TypeError; }
			 console.log(JSON.stringify(second));`
		);
		assert.strictEqual(result, true);
	});

	test('leaving a loop early lets the process exit with stdin still open', async () => {
		const child = node_spawn(
			process.execPath,
			[write_fixture(`for await (const chunk of stdin) { await stdout.write('got'); break; }`)],
			{ cwd: runner.sandbox.root, env: {}, stdio: ['pipe', 'pipe', 'pipe'] }
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
					 for await (const chunk of stdin) break;
					 const inheritor = spawn(globalThis.process.execPath, ['-e', 'process.stdin.pipe(process.stdout)'], {
						stdio: ['inherit', 'inherit', 'inherit'],
					 });
					 inheritor.on('spawn', () => { globalThis.process.stdout.write('ready'); });
					 await new Promise((resolve) => inheritor.on('close', resolve));`
				)
			],
			{ cwd: runner.sandbox.root, env: {}, stdio: ['pipe', 'pipe', 'inherit'] }
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

	test('a chunk that is not bytes is a `StdioError` `io_error`, whether from an encoding or a forged result', () => {
		const encoded = run_fixture_json(
			`globalThis.process.stdin.setEncoding('utf8');
			 let kind = null;
			 try { for await (const chunk of stdin) void chunk; } catch (e) { kind = [e.name, e.kind]; }
			 console.log(JSON.stringify(kind));`,
			{ input: 'hello\n' }
		);
		assert.deepEqual(encoded, ['StdioError', 'io_error']);
		const forged = run_fixture_json(
			`const iterator = stdin[Symbol.asyncIterator]();
			 Object.prototype.then = function (ok) { delete Object.prototype.then; ok({ done: false, value: 'forged' }); };
			 let kind = null;
			 try { await iterator.next(); } catch (e) { kind = [e.name, e.kind]; }
			 delete Object.prototype.then;
			 console.log(JSON.stringify(kind));`,
			{ input: 'hello\n' }
		);
		assert.deepEqual(forged, ['StdioError', 'io_error']);
	});

	test('`is_terminal` reports a pipe as no terminal', () => {
		assert.strictEqual(run_fixture_json('console.log(JSON.stringify(stdin.is_terminal));'), false);
	});
});

describe('stdin handle hardening', () => {
	test('no method or getter acts on a borrowed receiver', () => {
		const { seen, failures } = run_fixture_json(
			to_check_handles_body('{ stdin, input_iterator: stdin[Symbol.asyncIterator]() }')
		) as { seen: Array<string>; failures: Array<string> };
		assert.deepEqual(failures, []);
		for (const name of [
			'stdin.is_terminal',
			'stdin.Symbol(Symbol.asyncIterator)',
			'input_iterator.next',
			'input_iterator.return'
		]) {
			assert.include(seen, name);
		}
	});

	test('is frozen, with no thenable surface', () => {
		const result = run_fixture_json(
			`Object.prototype.then = function (ok) { ok('forged'); };
			 const awaited = await stdin;
			 delete Object.prototype.then;
			 console.log(JSON.stringify({ forged: awaited === 'forged', frozen: Object.isFrozen(stdin) }));`
		);
		assert.deepEqual(result, { forged: false, frozen: true });
	});

	test('`stdin.constructor` refuses without the key', () => {
		const result = run_fixture_json(
			`let refused = false;
			 try { new stdin.constructor(Symbol('forged')); } catch (e) { refused = e instanceof TypeError; }
			 console.log(JSON.stringify(refused));`
		);
		assert.strictEqual(result, true);
	});
});
