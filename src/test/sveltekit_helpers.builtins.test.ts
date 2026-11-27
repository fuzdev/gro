import { test, assert, vi, onTestFinished } from 'vitest';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { Logger } from '@fuzdev/fuz_util/log.ts';
import * as s from '@fuzdev/fuz_schema/schema.ts';

import { task as gen_task } from '#lib/gen.task.ts';
import { task as lint_task } from '#lib/lint.task.ts';
import { task as check_task } from '#lib/check.task.ts';
import { GRO_BUILTIN_TASK_GRANTS } from '#lib/gro_builtin_tasks.ts';
import { SVELTEKIT_ROUTE_META_DATA_PATH, SVELTEKIT_TSCONFIG_PATH } from '#lib/constants.ts';
import type { BuiltinTask } from '#lib/builtin_host.ts';

import { create_mock_builtin_context } from './test_helpers.ts';
import { create_mock_spawn } from './spawn_mocks.ts';
import { create_project_fs_paths } from './fs_test_helpers.ts';

// the builtins that read SvelteKit's generated files and sync them first when they're missing
const SYNCING = { gen: gen_task, lint: lint_task, check: check_task } as const;

// what each runs with in the project `dir` besides its defaults:
// `gen` resolving its inputs there, and `check` with every step off but `build`, which syncs first
const to_args = (dir: string): Record<keyof typeof SYNCING, object> => ({
	gen: { root_dirs: [dir] },
	lint: {},
	check: {
		typecheck: false,
		test: false,
		gen: false,
		format: false,
		package_json: false,
		lint: false,
		build: true,
		lockfile: false,
		install: false,
		workspace: false
	}
});

// a SvelteKit app, synced or not, with eslint declared so `lint` gets past its declaration check
const create_project = (synced: boolean): string => {
	const dir = mkdtempSync(join(tmpdir(), 'gro_builtin_sync_'));
	onTestFinished(() => rmSync(dir, { recursive: true }));
	writeFileSync(
		join(dir, 'package.json'),
		JSON.stringify({ name: 'x', devDependencies: { '@sveltejs/kit': '*', eslint: '*' } })
	);
	mkdirSync(join(dir, 'src'));
	mkdirSync(join(dir, 'node_modules'));
	if (synced) {
		for (const path of [SVELTEKIT_ROUTE_META_DATA_PATH, SVELTEKIT_TSCONFIG_PATH]) {
			mkdirSync(dirname(join(dir, path)), { recursive: true });
			writeFileSync(join(dir, path), '{}');
		}
	}
	return dir;
};

// runs the builtin, its args parsed with their defaults, in the project, its programs a mock spawn's
const run_in = async (
	name: keyof typeof SYNCING,
	synced: boolean,
	spawn = create_mock_spawn()
): Promise<{ spawn: typeof spawn; log: Logger }> => {
	vi.spyOn(Logger.prototype, 'info').mockImplementation(() => undefined);
	const task = SYNCING[name] as unknown as BuiltinTask;
	const dir = create_project(synced);
	const parsed = s.parse(task.Args!, to_args(dir)[name]);
	assert(parsed.ok, name);
	const ctx = await create_mock_builtin_context(task, parsed.value, {
		spawn,
		fs_paths: await create_project_fs_paths(dir)
	});
	// `lint` finds eslint and runs it, which the mock passes
	await task.run(ctx);
	return { spawn, log: ctx.log };
};

test("`gen`, `lint`, and `check` each grant themselves the project's `svelte-kit` and its tool directories", () => {
	for (const name of Object.keys(SYNCING)) {
		assert.deepEqual(
			GRO_BUILTIN_TASK_GRANTS[name as keyof typeof SYNCING]?.programs?.sveltekit_cli,
			{
				name: 'svelte-kit',
				local: true,
				pass_env: 'all',
				write: ['.svelte-kit', 'node_modules/$app', 'node_modules/.vite-temp']
			},
			name
		);
	}
});

test('`gen`, `lint`, and `check` sync an unsynced SvelteKit project through their own grant', async () => {
	for (const name of Object.keys(SYNCING) as Array<keyof typeof SYNCING>) {
		const { spawn } = await run_in(name, false);
		const syncs = spawn.calls.filter((call) => call.name === 'node_modules/.bin/svelte-kit');
		assert.strictEqual(syncs.length, 1, name);
		assert.deepEqual(syncs[0]!.args, ['sync'], name);
		assert.deepEqual(syncs[0]!.program_options, { pass_env: 'all' }, name);
	}
});

test('`gen`, `lint`, and `check` find no `svelte-kit` for a synced project', async () => {
	for (const name of Object.keys(SYNCING) as Array<keyof typeof SYNCING>) {
		const { spawn } = await run_in(name, true);
		assert.notInclude(spawn.finds, 'node_modules/.bin/svelte-kit', name);
	}
});

test('`gen`, `lint`, and `check` warn once that SvelteKit is declared but not installed', async () => {
	for (const name of Object.keys(SYNCING) as Array<keyof typeof SYNCING>) {
		const spawn = create_mock_spawn();
		spawn.available = (n) => n !== 'node_modules/.bin/svelte-kit' && n.startsWith('node_modules/');
		const { log } = await run_in(name, false, spawn);
		const warned = vi
			.mocked(log.warn)
			.mock.calls.filter((call) => String(call[0]).includes("SvelteKit's files weren't synced"));
		assert.strictEqual(warned.length, 1, name);
	}
});

test("a run's builtins that each sync each warn once, under their own label", async () => {
	const spawn = create_mock_spawn();
	spawn.available = (n) => n !== 'node_modules/.bin/svelte-kit' && n.startsWith('node_modules/');
	const dir = create_project(false);
	// one run's boundary, shared by every task it runs
	const fs_paths = await create_project_fs_paths(dir);
	for (const name of ['gen', 'lint'] as const) {
		const task = SYNCING[name] as unknown as BuiltinTask;
		const parsed = s.parse(task.Args!, to_args(dir)[name]);
		assert(parsed.ok, name);
		const ctx = await create_mock_builtin_context(task, parsed.value, { spawn, fs_paths });
		await task.run(ctx);
		const warned = vi
			.mocked(ctx.log.warn)
			.mock.calls.filter((call) => String(call[0]).includes("SvelteKit's files weren't synced"));
		assert.strictEqual(warned.length, 1, name);
	}
});
