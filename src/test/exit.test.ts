import { describe, test, assert, vi } from 'vitest';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { to_lib_url, use_fixture_runner } from './host_fixture_helpers.ts';

vi.setConfig({ testTimeout: 30_000 });

const SPAWN_URL = to_lib_url('spawn.ts');

const runner = use_fixture_runner(
	`import {exit, set_exit_code} from ${JSON.stringify(to_lib_url('exit.ts'))};`
);
const { run_fixture, run_fixture_json } = runner;

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

describe('exit', () => {
	const CODE_CHECKS = `
		const bad = [256, -1, 1.5, NaN, Infinity, '1', null, true, {}, [], 1n, Symbol('s')];
		let called = false;
		bad.push({ then(ok) { called = true; ok(1); } });
		bad.push({ [Symbol.toPrimitive]() { called = true; return 1; } });
		let refused = 0;
		for (const code of bad) {
			try { exit(code); } catch (e) { if (e instanceof TypeError) refused++; }
			try { set_exit_code(code); } catch (e) { if (e instanceof TypeError) refused++; }
		}
		console.log(JSON.stringify({ refused, total: bad.length * 2, called }));
	`;

	test('exits with the code', () => {
		assert.strictEqual(run_fixture('exit(5);').status, 5);
		assert.strictEqual(run_fixture('exit(0);').status, 0);
		assert.strictEqual(run_fixture('exit(255);').status, 255);
	});

	test('with no code uses `set_exit_code`, else 0', () => {
		assert.strictEqual(run_fixture('set_exit_code(7); exit();').status, 7);
		assert.strictEqual(run_fixture('exit();').status, 0);
	});

	test('`set_exit_code` applies when the process ends on its own', () => {
		assert.strictEqual(run_fixture('set_exit_code(9);').status, 9);
	});

	test('refuses every other code with a TypeError, running none of the code it was given', () => {
		const result = run_fixture_json(CODE_CHECKS) as Record<string, unknown>;
		assert.strictEqual(result.refused, result.total);
		assert.strictEqual(result.called, false);
	});

	test('runs the spawn module`s child cleanup', async () => {
		const bin = join(runner.sandbox.base, 'bin');
		mkdirSync(bin);
		const ready = join(runner.sandbox.base, 'pid');
		const sleeper = join(bin, 'sleeper');
		// the pid file appears whole, by rename: the fixture exits as soon as it sees the file,
		// and a child killed between creating it and writing its pid would leave it empty
		writeFileSync(
			sleeper,
			`#!/bin/sh\necho $$ > '${ready}.tmp'\nmv '${ready}.tmp' '${ready}'\nexec sleep 100\n`,
			{ mode: 0o755 }
		);
		const result = run_fixture(
			`import {spawn} from ${JSON.stringify(SPAWN_URL)};
			 import {existsSync} from 'node:fs';
			 await spawn.program('sleeper').start([], {stdio: 'pipe'});
			 for (let i = 0; i < 2000 && !existsSync(${JSON.stringify(ready)}); i++) {
				await new Promise((r) => setTimeout(r, 5));
			 }
			 exit(3);`,
			{ env: { PATH: `${bin}:/usr/bin:/bin` } }
		);
		assert.strictEqual(result.status, 3, result.stderr);
		assert.ok(existsSync(ready), 'the child published no pid');
		const pid_text = readFileSync(ready, 'utf8');
		const pid = Number(pid_text.trim());
		// `process.kill` takes 0 and negative pids as process groups, the test's own among them
		assert.ok(Number.isInteger(pid) && pid > 1, `the pid file holds ${JSON.stringify(pid_text)}`);
		await assert_gone(pid);
	});

	test('`exit` and `set_exit_code` are frozen functions, no constructors, taking no receiver', () => {
		const result = run_fixture(
			`const frozen = [exit, set_exit_code].every(Object.isFrozen);
			 let constructed = 0;
			 for (const fn of [exit, set_exit_code]) {
				try { new fn(3); constructed++; } catch (e) { if (!(e instanceof TypeError)) constructed++; }
			 }
			 if (!frozen || constructed !== 0) exit(1);
			 set_exit_code.call({}, 4);
			 exit.call(null);`
		);
		assert.strictEqual(result.status, 4, result.stderr);
	});
});
