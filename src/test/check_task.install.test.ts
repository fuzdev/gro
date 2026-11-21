import { test, assert, vi, onTestFinished } from 'vitest';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { run_check, type Args, type CheckOptions } from '#lib/check.task.ts';
import { GRO_BUILTIN_TASK_GRANTS } from '#lib/gro_builtin_tasks.ts';
import { is_task_error } from '#lib/task_error.ts';

import { create_mock_logger } from './test_helpers.ts';
import { create_project_fs_paths } from './fs_test_helpers.ts';

// every step off but the ones a test turns on
const NONE = {
	typecheck: false,
	test: false,
	gen: false,
	format: false,
	package_json: false,
	lint: false,
	build: false,
	lockfile: false,
	install: false,
	workspace: false
} as unknown as Args;

const PACKAGE_JSON = '{"name": "x", "version": "0.0.0", "dependencies": {"a": "^1.0.0"}}\n';

// a project whose lockfile locks `a` at 1.0.0, installed at `installed`, or with no lockfile
const create_project = (installed: string | null): string => {
	const dir = mkdtempSync(join(tmpdir(), 'gro_check_install_'));
	onTestFinished(() => rmSync(dir, { recursive: true }));
	writeFileSync(join(dir, 'package.json'), PACKAGE_JSON);
	if (installed !== null) {
		const entry = { version: '1.0.0', resolved: 'r', integrity: 'i' };
		writeFileSync(
			join(dir, 'package-lock.json'),
			JSON.stringify({
				lockfileVersion: 3,
				packages: { '': { dependencies: { a: '^1.0.0' } }, 'node_modules/a': entry }
			})
		);
		mkdirSync(join(dir, 'node_modules/a'), { recursive: true });
		writeFileSync(
			join(dir, 'node_modules/.package-lock.json'),
			JSON.stringify({ lockfileVersion: 3, packages: { 'node_modules/a': entry } })
		);
		writeFileSync(join(dir, 'node_modules/a/package.json'), JSON.stringify({ version: installed }));
	}
	return dir;
};

const run = async (
	dir: string,
	args: Partial<Args>,
	options: Partial<CheckOptions> = {}
): Promise<{
	error: unknown;
	invoke_task: ReturnType<typeof vi.fn>;
	log: ReturnType<typeof create_mock_logger>;
}> => {
	const invoke_task = vi.fn();
	const log = create_mock_logger();
	let error: unknown;
	try {
		await run_check(
			{ ...NONE, ...args },
			{ map_package_json: null, raw_rest_args: [], ...options },
			{
				fs_paths: await create_project_fs_paths(dir),
				env: { get: () => undefined },
				programs: { find: () => null },
				invoke_task: invoke_task as never,
				log
			}
		);
	} catch (e) {
		error = e;
	}
	return { error, invoke_task, log };
};

const to_infos = (log: ReturnType<typeof create_mock_logger>): Array<string> =>
	vi.mocked(log.info).mock.calls.map((call) => String(call[0]));

test('`check` fails naming `npm i` when the install does not match the lockfile', async () => {
	const { error } = await run(create_project('1.0.1'), { lockfile: true });
	assert(is_task_error(error));
	assert.strictEqual(
		error.message,
		"the install doesn't match package.json and package-lock.json: run `npm i`, " +
			'or pass `--no-lockfile` to skip this check: ' +
			'node_modules/a is "1.0.1" installed, "1.0.0" locked'
	);
});

test('`check` passes an install that matches, saying so', async () => {
	const { error, log } = await run(create_project('1.0.0'), { lockfile: true });
	assert.strictEqual(error, undefined);
	assert.include(to_infos(log), 'check passed for the install');
});

test('with no package-lock.json, `check` says at info level the install was not checked', async () => {
	const { error, log } = await run(create_project(null), { lockfile: true });
	assert.strictEqual(error, undefined);
	assert.include(to_infos(log), "the install wasn't checked: the project has no package-lock.json");
});

test('`--no-lockfile` skips the install check', async () => {
	const { error, log } = await run(create_project('1.0.1'), { lockfile: false });
	assert.strictEqual(error, undefined);
	assert.notInclude(to_infos(log).join('\n'), 'install');
});

test('`check` invokes `sync` only under `--install`, then never to write `package.json`', async () => {
	const dir = create_project('1.0.0');
	assert.deepEqual((await run(dir, {})).invoke_task.mock.calls, []);
	assert.deepEqual((await run(dir, { install: true })).invoke_task.mock.calls, [
		['sync', { install: true, gen: false, package_json: false }]
	]);
	// CI installs in its own setup
	assert.deepEqual((await run(dir, { install: true, workspace: true })).invoke_task.mock.calls, []);
	assert.deepEqual(GRO_BUILTIN_TASK_GRANTS.check?.invokes?.sync, { when: 'install' });
});

test('a drifted `package.json` fails `check` naming `gro sync`, and nothing writes it', async () => {
	const dir = create_project('1.0.0');
	const { error, invoke_task } = await run(
		dir,
		{ package_json: true },
		{ map_package_json: (package_json) => ({ ...package_json, description: 'drifted' }) }
	);
	assert(is_task_error(error));
	assert.strictEqual(error.message, 'package.json is out of date, run `gro sync` to update it');
	assert.deepEqual(invoke_task.mock.calls, []);
	assert.strictEqual(readFileSync(join(dir, 'package.json'), 'utf8'), PACKAGE_JSON);
});

test('`check` skips a linked package with an info note naming it', async () => {
	const dir = create_project('1.0.0');
	rmSync(join(dir, 'node_modules/a'), { recursive: true });
	mkdirSync(join(dir, 'sibling'));
	writeFileSync(join(dir, 'sibling/package.json'), '{"version": "9.9.9"}');
	symlinkSync('../sibling', join(dir, 'node_modules/a'));
	const { error, log } = await run(dir, { lockfile: true });
	assert.strictEqual(error, undefined);
	assert.include(
		to_infos(log),
		'the install check skipped the linked packages, which follow their source: node_modules/a'
	);
});

test('`check` names at most ten install mismatches, counting the rest', async () => {
	const dir = create_project('1.0.0');
	// twelve specs `package.json` declares and the lockfile's root entry doesn't record
	const dependencies: Record<string, string> = { a: '^1.0.0' };
	for (let i = 0; i < 12; i++) dependencies[`extra_${String(i).padStart(2, '0')}`] = '^1.0.0';
	writeFileSync(join(dir, 'package.json'), JSON.stringify({ name: 'x', dependencies }));
	const { error } = await run(dir, { lockfile: true });
	assert(is_task_error(error));
	assert.strictEqual(error.message.split('; ').length, 11);
	assert.include(error.message, 'extra_09');
	assert.notInclude(error.message, 'extra_10');
	assert.isTrue(error.message.endsWith('; and 2 more'));
});
