import { assert, beforeEach, afterEach } from 'vitest';
import { spawnSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { paths } from '#lib/paths.ts';
import { create_fs_sandbox, remove_fs_sandbox, type FsSandbox } from './fs_test_helpers.ts';

/*

Helpers for the tests of the env, args, exit, stdio, cwd, pid, and runtime_version host modules,
whose cases that exit, read a chosen environment, or break a pipe run in a child through a fixture
script, under an environment of fake names only, so nothing exits the test process, signals
outside a child, or reads the ambient environment.

*/

/**
 * The file URL of a module in `src/lib`, as a fixture script imports it.
 * @param name - the module's path in `src/lib`, like `env.ts`
 */
export const to_lib_url = (name: string): string => new URL(join(paths.lib, name), 'file:').href;

export interface FixtureRunOptions {
	env?: Record<string, string>;
	input?: string;
}

export interface FixtureResult {
	status: number | null;
	stdout: string;
	stderr: string;
}

export interface FixtureRunner {
	/**
	 * The sandbox of the current test, made fresh for each.
	 */
	readonly sandbox: FsSandbox;
	/**
	 * Runs `node` with `args` in the sandbox's root.
	 */
	run_node: (args: Array<string>, options?: FixtureRunOptions) => FixtureResult;
	/**
	 * Writes a fixture script of the header's imports and `body`, returning its path.
	 */
	write_fixture: (body: string) => string;
	/**
	 * Writes and runs a fixture script with `args` after it.
	 */
	run_fixture: (body: string, options?: FixtureRunOptions, args?: Array<string>) => FixtureResult;
	/**
	 * Runs a fixture script that exits `0`, parsing its stdout as JSON.
	 */
	run_fixture_json: (body: string, options?: FixtureRunOptions, args?: Array<string>) => unknown;
}

/**
 * Registers a sandbox for each test of the calling file and returns fixture runners over it.
 * @param header - the imports every fixture script starts with
 */
export const use_fixture_runner = (header: string): FixtureRunner => {
	let sandbox: FsSandbox;

	beforeEach(async () => {
		sandbox = await create_fs_sandbox();
	});

	afterEach(() => {
		remove_fs_sandbox(sandbox);
	});

	const run_node = (args: Array<string>, options: FixtureRunOptions = {}): FixtureResult => {
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
		writeFileSync(path, `${header}\n${body}\n`);
		return path;
	};

	const run_fixture = (
		body: string,
		options: FixtureRunOptions = {},
		args: Array<string> = []
	): FixtureResult => run_node([write_fixture(body), ...args], options);

	const run_fixture_json = (
		body: string,
		options: FixtureRunOptions = {},
		args: Array<string> = []
	): unknown => {
		const result = run_fixture(body, options, args);
		assert.strictEqual(result.status, 0, result.stderr);
		return JSON.parse(result.stdout);
	};

	return {
		get sandbox() {
			return sandbox;
		},
		run_node,
		write_fixture,
		run_fixture,
		run_fixture_json
	};
};

/**
 * A fixture body checking `handles`, an object literal's source naming each handle, and printing
 * `{seen, failures}`: every method and getter on a handle's prototype refuses a borrowed receiver
 * with a `TypeError`, its constructor refuses a forged key, and the handle and its prototype are
 * frozen and null-prototype.
 */
export const to_check_handles_body = (handles: string): string => `
	const fakes = () => [{}, Object.create(null), { pid: 1 }, null, undefined, globalThis.process];
	const handles = ${handles};
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
			const args = { narrow: [['GRO_FAKE_A']], get: ['GRO_FAKE_A'] }[String(key)] ?? [];
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
	console.log(JSON.stringify({ seen, failures }));`;
