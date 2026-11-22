import { test, assert, describe, vi } from 'vitest';

import {
	SVELTEKIT_PROGRAM_OPTIONS,
	sveltekit_sync_if_obviously_needed
} from '#lib/sveltekit_helpers.ts';
import { to_cli_finder } from '#lib/cli.ts';
import {
	SVELTEKIT_DEP_NAME,
	SVELTEKIT_DEV_DIRNAME,
	SVELTEKIT_ROUTE_META_DATA_PATH,
	SVELTEKIT_TSCONFIG_PATH
} from '#lib/constants.ts';

import { create_mock_fs_dir, create_mock_project_fs_paths, mock_tree } from './fs_mocks.ts';
import { create_mock_spawn } from './spawn_mocks.ts';

const DECLARED = JSON.stringify({ devDependencies: { [SVELTEKIT_DEP_NAME]: '3' } });

const to_project = (tree: Record<string, string>) => {
	const project = create_mock_fs_dir('/project');
	mock_tree(project, tree);
	return create_mock_project_fs_paths(project);
};

const count_syncs = (spawn: ReturnType<typeof create_mock_spawn>): number =>
	spawn.calls.filter((call) => call.method === 'run' && call.args[0] === 'sync').length;

describe('sveltekit_sync_if_obviously_needed', () => {
	test('a synced project reads no package.json', async () => {
		const fs_paths = to_project({
			'package.json': DECLARED,
			[SVELTEKIT_ROUTE_META_DATA_PATH]: '{}',
			[SVELTEKIT_TSCONFIG_PATH]: '{}'
		});
		const open_path = vi.spyOn(fs_paths, 'open_path');
		const spawn = create_mock_spawn();
		await sveltekit_sync_if_obviously_needed(
			fs_paths,
			to_cli_finder(spawn, SVELTEKIT_PROGRAM_OPTIONS),
			{ warn: vi.fn() }
		);
		assert.strictEqual(count_syncs(spawn), 0);
		for (const [path] of open_path.mock.calls) {
			assert.notInclude(path, 'package.json');
		}
	});

	// either file missing means a sync, `npm install` wiping only the generated tsconfig
	test.each([
		['only the route records in `.svelte-kit`', SVELTEKIT_ROUTE_META_DATA_PATH],
		['only the generated tsconfig', SVELTEKIT_TSCONFIG_PATH]
	])('syncs a declared project with %s', async (_name, present) => {
		const spawn = create_mock_spawn();
		const warn = vi.fn();
		await sveltekit_sync_if_obviously_needed(
			to_project({ 'package.json': DECLARED, [present]: '{}' }),
			to_cli_finder(spawn, SVELTEKIT_PROGRAM_OPTIONS),
			{ warn }
		);
		assert.strictEqual(count_syncs(spawn), 1);
		assert.deepEqual(warn.mock.calls, []);
	});

	// a `.svelte-kit` that exists unsynced, as a tool that makes the directory leaves it,
	// or a sync that stopped before rewriting its route records
	test.each([
		['empty', `${SVELTEKIT_DEV_DIRNAME}/`],
		['holding files no sync wrote', `${SVELTEKIT_DEV_DIRNAME}/output/x`],
		[
			'holding route types but no route records',
			`${SVELTEKIT_DEV_DIRNAME}/types/src/routes/$types.d.ts`
		]
	])('syncs a declared project whose `.svelte-kit` is %s', async (_name, entry) => {
		const spawn = create_mock_spawn();
		await sveltekit_sync_if_obviously_needed(
			to_project({ 'package.json': DECLARED, [entry]: '', [SVELTEKIT_TSCONFIG_PATH]: '{}' }),
			to_cli_finder(spawn, SVELTEKIT_PROGRAM_OPTIONS),
			{ warn: vi.fn() }
		);
		assert.strictEqual(count_syncs(spawn), 1);
	});
});

describe('sveltekit_sync_if_obviously_needed without the CLI', () => {
	const run = async (tree: Record<string, string>): Promise<Array<unknown>> => {
		const spawn = create_mock_spawn();
		spawn.available = () => false;
		const warn = vi.fn();
		await sveltekit_sync_if_obviously_needed(
			to_project(tree),
			to_cli_finder(spawn, SVELTEKIT_PROGRAM_OPTIONS),
			{ warn }
		);
		assert.strictEqual(count_syncs(spawn), 0);
		return warn.mock.calls;
	};

	test('a declared project with a `node_modules` missing the CLI warns once', async () => {
		assert.deepEqual(await run({ 'package.json': DECLARED, 'node_modules/x/package.json': '{}' }), [
			[
				"@sveltejs/kit is declared but `svelte-kit` isn't in node_modules/.bin," +
					" so SvelteKit's files weren't synced - run `npm i`"
			]
		]);
	});

	test('a fresh clone with no `node_modules` skips quietly, so an install can run', async () => {
		assert.deepEqual(await run({ 'package.json': DECLARED }), []);
	});

	test('a project not declaring SvelteKit, or already synced, never warns', async () => {
		assert.deepEqual(await run({ 'package.json': '{}', 'node_modules/x/package.json': '{}' }), []);
		assert.deepEqual(
			await run({
				'package.json': DECLARED,
				[SVELTEKIT_ROUTE_META_DATA_PATH]: '{}',
				[SVELTEKIT_TSCONFIG_PATH]: '{}'
			}),
			[]
		);
	});
});

test("`to_cli_finder` finds only in the project's `node_modules/.bin`, with its options", () => {
	const spawn = create_mock_spawn();
	const recorded: Array<unknown> = [];
	const find = spawn.find.bind(spawn);
	spawn.find = (name, options) => {
		recorded.push(options);
		return find(name, options);
	};
	const finder = to_cli_finder(spawn, SVELTEKIT_PROGRAM_OPTIONS);
	// only a copy on `PATH`, which never runs in place of the project's
	spawn.available = (name) => name === 'svelte-kit';
	assert.strictEqual(finder.find('svelte-kit'), null);
	spawn.available = (name) => name === 'node_modules/.bin/svelte-kit';
	assert.ok(finder.find('svelte-kit'));
	assert.deepEqual(spawn.finds, ['node_modules/.bin/svelte-kit', 'node_modules/.bin/svelte-kit']);
	assert.deepEqual(recorded, [SVELTEKIT_PROGRAM_OPTIONS, SVELTEKIT_PROGRAM_OPTIONS]);
});
