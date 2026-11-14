import { describe, test, assert, vi } from 'vitest';
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { spawn_with_loader_args } from '#lib/gro_helpers.ts';
import { paths } from '#lib/paths.ts';
import { to_lib_url, use_fixture_runner } from './host_fixture_helpers.ts';
import { LOADER_SOURCE_PATH } from './test_helpers.ts';

vi.setConfig({ testTimeout: 30_000 });

const MODULE_URL = to_lib_url('args.ts');

const runner = use_fixture_runner(`import {args} from ${JSON.stringify(MODULE_URL)};`);
const { run_node, write_fixture, run_fixture_json } = runner;

describe('args', () => {
	const ARGS_FIXTURE = 'console.log(JSON.stringify(args));';

	test('are the arguments after the script under bare node, frozen', () => {
		const result = run_fixture_json(ARGS_FIXTURE, {}, ['a', '--b', '-c', '--', 'd']);
		assert.deepEqual(result, ['a', '--b', '-c', '--', 'd']);
		assert.ok(
			run_fixture_json('console.log(Object.isFrozen(args));') === true,
			'the array is frozen'
		);
	});

	test('are the arguments under the loader, as `gro run` starts the script', () => {
		const script = write_fixture(ARGS_FIXTURE);
		const { status, stdout, stderr } = run_node(
			spawn_with_loader_args(LOADER_SOURCE_PATH, script, ['--help', 'a', '--', 'b'], undefined),
			{ env: { PATH: '/usr/bin:/bin' } }
		);
		assert.strictEqual(status, 0, stderr);
		assert.deepEqual(JSON.parse(stdout.trim()), ['--help', 'a', '--', 'b']);
	});

	test('under the CLI they are the CLI args, the task name included', () => {
		const task = join(runner.sandbox.root, 'args.task.ts');
		writeFileSync(
			task,
			`import {args} from ${JSON.stringify(MODULE_URL)};
			 export const task = {summary: 'x', run: () => { console.log('ARGS' + JSON.stringify(args)); }};`
		);
		const { status, stdout, stderr } = run_node(
			spawn_with_loader_args(
				LOADER_SOURCE_PATH,
				join(paths.lib, 'invoke.ts'),
				['task', 'args.task.ts', '--flag', 'v', '--', 'other'],
				undefined
			),
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
			`import(${JSON.stringify(MODULE_URL)}).then((m) => console.log(JSON.stringify(m.args)))`,
			'a',
			'b'
		]);
		assert.strictEqual(result.status, 0, result.stderr);
		assert.deepEqual(JSON.parse(result.stdout), ['a', 'b']);
	});

	test('a script read from stdin as `-` keeps its arguments', () => {
		const result = run_node(['-', 'a', 'b'], {
			input: `import(${JSON.stringify(MODULE_URL)}).then((m) => console.log(JSON.stringify(m.args)));`
		});
		assert.strictEqual(result.status, 0, result.stderr);
		assert.deepEqual(JSON.parse(result.stdout), ['a', 'b']);
	});

	const WORKER_POST_ARGS = `import(${JSON.stringify(MODULE_URL)}).then((m) => require('node:worker_threads').parentPort.postMessage(m.args));`;

	test('an eval worker has its own arguments, without the eval marker', () => {
		const result = run_fixture_json(
			`import {Worker} from 'node:worker_threads';
			 const worker = new Worker(${JSON.stringify(WORKER_POST_ARGS)}, {eval: true, argv: ['a', 'b']});
			 worker.on('message', (args) => console.log(JSON.stringify(args)));`
		);
		assert.deepEqual(result, ['a', 'b']);
	});

	test('a file worker started from a `node -e` parent has its own arguments', () => {
		const worker_path = join(runner.sandbox.base, 'worker.cjs');
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
			 const awaited = await args;
			 console.log(JSON.stringify({
				awaited,
				is_array: Array.isArray(args),
				frozen: Object.isFrozen(args),
				proto: Object.getPrototypeOf(args) === Array.prototype,
			 }));`
		);
		assert.deepEqual(result, { awaited: 'forged', is_array: true, frozen: true, proto: true });
	});
});
