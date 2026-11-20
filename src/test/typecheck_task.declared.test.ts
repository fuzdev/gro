import { test, assert, vi } from 'vitest';

import { run_typecheck } from '#lib/typecheck.task.ts';
import { is_task_error } from '#lib/task_error.ts';
import { is_fs_error } from '#lib/fs_error.ts';
import type { ProcessOutput } from '#lib/stdout.ts';
import type { SpawnProgram } from '#lib/spawn.ts';

import { create_mock_logger } from './test_helpers.ts';
import { create_mock_fs_dir, create_mock_project_fs_paths, mock_tree } from './fs_mocks.ts';

type Role = 'sveltekit_cli' | 'svelte_check_cli' | 'typescript_cli';

const create_program = (name: string): SpawnProgram =>
	({
		name,
		path: '/bin/' + name,
		run: vi.fn(() => Promise.resolve({ kind: 'exited', code: 0 }))
	}) as unknown as SpawnProgram;

/**
 * Runs `typecheck` over a mock project with `package_json`'s dependencies and the files of `tree`,
 * each role in `found` found, as one on `PATH` would be.
 */
const run = async (
	package_json: object,
	tree: Record<string, string> = {},
	found: ReadonlyArray<Role> = ['svelte_check_cli', 'typescript_cli'],
	raw_rest_args: ReadonlyArray<string> = []
): Promise<{ error: unknown; finds: Array<Role>; programs: Map<Role, SpawnProgram> }> => {
	const project = create_mock_fs_dir('/project');
	mock_tree(project, {
		'package.json': JSON.stringify({ name: 'a', version: '0', ...package_json }),
		...tree
	});
	const programs: Map<Role, SpawnProgram> = new Map();
	const finds: Array<Role> = [];
	const find = (role: Role): SpawnProgram | null => {
		finds.push(role);
		if (!found.includes(role)) return null;
		const program = create_program(role);
		programs.set(role, program);
		return program;
	};
	let error: unknown = null;
	try {
		await run_typecheck(
			{ path_replacement: '.', cwd: '/project' },
			{ raw_rest_args },
			{
				fs_paths: create_mock_project_fs_paths(project),
				programs: { find },
				stdout: {} as ProcessOutput,
				stderr: {} as ProcessOutput,
				log: create_mock_logger()
			}
		);
	} catch (e) {
		error = e;
	}
	return { error, finds, programs };
};

const assert_task_error = (error: unknown, message: string): void => {
	assert(is_task_error(error));
	assert.strictEqual(error.message, message);
};

test('svelte-check declared but found nowhere gives the install hint', async () => {
	for (const key of ['devDependencies', 'dependencies']) {
		const { error, finds } = await run({ [key]: { 'svelte-check': '4' } }, {}, ['typescript_cli']);
		assert_task_error(error, 'svelte-check is a dependency but not installed; run `npm i`?');
		assert.deepEqual(finds, ['svelte_check_cli']);
	}
});

test('svelte-check undeclared with Svelte files fails naming them and the fix, running nothing', async () => {
	const { error, finds } = await run(
		{ devDependencies: { typescript: '6' } },
		{ 'src/lib/A.svelte': '', 'src/routes/+page.svelte': '', 'src/lib/a.ts': '' }
	);
	assert_task_error(
		error,
		"found 2 Svelte files in src but svelte-check isn't declared in package.json: " +
			'src/lib/A.svelte, src/routes/+page.svelte - add svelte-check to devDependencies, remove the Svelte files, or pass --no-typecheck to gro check'
	);
	// neither checker is looked up, so one found on `PATH` never runs
	assert.deepEqual(finds, []);
});

test('svelte-check only peered counts as undeclared', async () => {
	const { error } = await run(
		{ peerDependencies: { 'svelte-check': '4' }, devDependencies: { typescript: '6' } },
		{ 'src/A.svelte': '' }
	);
	assert_task_error(
		error,
		"found 1 Svelte file in src but svelte-check isn't declared in package.json: " +
			'src/A.svelte - add svelte-check to devDependencies, remove the Svelte files, or pass --no-typecheck to gro check'
	);
});

test('a long list of Svelte files is capped, with the count of the rest', async () => {
	const tree = Object.fromEntries(Array.from({ length: 7 }, (_, i) => [`src/F${i}.svelte`, '']));
	const { error } = await run({ devDependencies: { typescript: '6' } }, tree);
	assert(is_task_error(error));
	assert.include(error.message, 'found 7 Svelte files in src');
	assert.include(error.message, 'src/F4.svelte');
	assert.notInclude(error.message, 'src/F5.svelte');
	assert.include(error.message, 'and 2 more');
});

test('Svelte files outside `src/` or under `node_modules` or `.git` leave tsc', async () => {
	const { error, finds, programs } = await run(
		{ devDependencies: { typescript: '6' } },
		{
			'A.svelte': '',
			'test/B.svelte': '',
			'src/node_modules/x/C.svelte': '',
			'src/.git/D.svelte': '',
			'src/a.ts': ''
		}
	);
	assert.strictEqual(error, null);
	assert.deepEqual(finds, ['typescript_cli']);
	assert.deepEqual(vi.mocked(programs.get('typescript_cli')!.run).mock.calls[0]![0], ['--noEmit']);
});

test('with no Svelte files, typescript declared and found runs tsc', async () => {
	for (const key of ['devDependencies', 'dependencies']) {
		const { error, finds } = await run({ [key]: { typescript: '6' } });
		assert.strictEqual(error, null);
		assert.deepEqual(finds, ['typescript_cli']);
	}
});

test('typescript declared but found nowhere gives the install hint', async () => {
	const { error } = await run({ devDependencies: { typescript: '6' } }, {}, ['svelte_check_cli']);
	assert_task_error(error, 'typescript is a dependency but not installed; run `npm i`?');
});

test('neither checker declared fails saying what to add, running nothing', async () => {
	for (const package_json of [{}, { peerDependencies: { typescript: '6' } }]) {
		const { error, finds } = await run(package_json);
		assert_task_error(error, 'no typechecker found, add typescript to devDependencies');
		assert.deepEqual(finds, []);
	}
});

test('a failed search for Svelte files is a `TaskError` with the fs error as its cause', async () => {
	const { error } = await run({ devDependencies: { typescript: '6' } }, { src: 'a file' });
	assert(is_task_error(error));
	assert.include(
		error.message,
		"failed to search src for Svelte files, as svelte-check isn't declared"
	);
	assert(is_fs_error(error.cause));
});

test('svelte-check declared starts it without searching for Svelte files', async () => {
	// `src` as a file would fail a search, and a `.svelte` file would fail an undeclared checker
	const trees: Array<Record<string, string>> = [{ 'src/A.svelte': '' }, { src: 'a file' }];
	for (const tree of trees) {
		const project = create_mock_fs_dir('/project');
		mock_tree(project, {
			'package.json': JSON.stringify({
				name: 'a',
				version: '0',
				devDependencies: { 'svelte-check': '4' }
			}),
			...tree
		});
		const started = new Error('svelte-check started');
		const svelte_check = {
			...create_program('svelte_check_cli'),
			start: vi.fn(() => Promise.reject(started))
		} as unknown as SpawnProgram;
		const finds: Array<Role> = [];
		let error: unknown = null;
		try {
			await run_typecheck(
				{ path_replacement: '.', cwd: '/project' },
				{ raw_rest_args: [] },
				{
					fs_paths: create_mock_project_fs_paths(project),
					programs: {
						find: (role: Role) => {
							finds.push(role);
							return role === 'svelte_check_cli' ? svelte_check : null;
						}
					},
					stdout: {} as ProcessOutput,
					stderr: {} as ProcessOutput,
					log: create_mock_logger()
				}
			);
		} catch (e) {
			error = e;
		}
		assert.strictEqual(error, started);
		assert.deepEqual(finds, ['svelte_check_cli']);
	}
});

test('a `.svelte.ts` or `.svelte.js` module alone leaves tsc, as tsc checks it', async () => {
	const { error, finds } = await run(
		{ devDependencies: { typescript: '6' } },
		{ 'src/lib/state.svelte.ts': '', 'src/lib/x.svelte.js': '' }
	);
	assert.strictEqual(error, null);
	assert.deepEqual(finds, ['typescript_cli']);
});

test('args forwarded to an undeclared svelte-check fail, running nothing', async () => {
	const { error, finds } = await run({ devDependencies: { typescript: '6' } }, {}, undefined, [
		'--',
		'svelte-check',
		'--threshold',
		'error'
	]);
	assert_task_error(
		error,
		"args are forwarded to svelte-check, which isn't declared in package.json" +
			' - add svelte-check to devDependencies, or drop the `-- svelte-check` section'
	);
	assert.deepEqual(finds, []);
});

test('an empty `-- svelte-check` section or a `-- tsc` one leaves tsc', async () => {
	for (const raw_rest_args of [
		['--', 'svelte-check'],
		['--', 'tsc', '--strict']
	]) {
		const { error, finds } = await run(
			{ devDependencies: { typescript: '6' } },
			{},
			undefined,
			raw_rest_args
		);
		assert.strictEqual(error, null);
		assert.deepEqual(finds, ['typescript_cli']);
	}
});

test('args forwarded to tsc with svelte-check declared fail, running nothing', async () => {
	const { error, finds } = await run(
		{ devDependencies: { 'svelte-check': '4', typescript: '6' } },
		{},
		undefined,
		['--', 'tsc', '--strict']
	);
	assert_task_error(
		error,
		"args are forwarded to tsc, which doesn't run as svelte-check is declared" +
			' - drop the `-- tsc` section, or forward to svelte-check with a `-- svelte-check` one'
	);
	assert.deepEqual(finds, []);
});

test('an empty `-- tsc` section with svelte-check declared goes on to svelte-check', async () => {
	const { error, finds } = await run(
		{ devDependencies: { 'svelte-check': '4', typescript: '6' } },
		{},
		['typescript_cli'],
		['--', 'tsc']
	);
	assert_task_error(error, 'svelte-check is a dependency but not installed; run `npm i`?');
	assert.deepEqual(finds, ['svelte_check_cli']);
});
