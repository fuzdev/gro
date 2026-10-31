import { describe, test, assert, afterEach, vi } from 'vitest';
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';

import {
	task_registry_find,
	task_registry_find_builtins,
	task_registry_render_builtins,
	task_registry_render_project,
	gen_task_registry,
	type TaskRegistryEntry
} from '#lib/task_registry.ts';
import { create_gen_context } from '#lib/gen.ts';
import { gen_inputs_match } from '#lib/gen_helpers.ts';
import { create_empty_gro_config } from '#lib/gro_config.ts';
import { paths } from '#lib/paths.ts';
import { to_builtin_task_id, type InvokeTask } from '#lib/task.ts';
import { GRO_BUILTIN_TASK_SUMMARIES } from '#lib/gro_builtin_tasks.ts';

import { create_mock_logger, create_mock_timings } from './test_helpers.ts';

let dirs: Array<string> = [];

afterEach(() => {
	for (const dir of dirs) rmSync(dir, { recursive: true, force: true });
	dirs = [];
});

const create_project = (files: Array<string>): string => {
	const dir = mkdtempSync(join(tmpdir(), 'gro_task_registry_'));
	dirs.push(dir);
	for (const file of files) {
		const id = join(dir, file);
		mkdirSync(dirname(id), { recursive: true });
		writeFileSync(id, 'export const task = {run: () => {}};\n');
	}
	return dir;
};

const to_names = (entries: Array<TaskRegistryEntry>): Array<string> => entries.map((e) => e.name);

describe('task_registry_find', () => {
	const { search_filters } = create_empty_gro_config();

	test('names tasks by their paths relative to the root, sorted', async () => {
		const dir = create_project([
			'src/lib/greet.task.ts',
			'src/lib/db/migrate.task.ts',
			'src/lib/not_a_task.ts',
			'src/test/fixtures/update.task.ts',
			'outside.task.ts'
		]);
		const entries = await task_registry_find(join(dir, 'src'), search_filters, dir);
		assert.deepEqual(to_names(entries), [
			'src/lib/db/migrate.task.ts',
			'src/lib/greet.task.ts',
			'src/test/fixtures/update.task.ts'
		]);
	});

	test('a subdirectory keeps names relative to the root', async () => {
		const dir = create_project(['src/lib/greet.task.ts', 'src/test/x.task.ts']);
		const entries = await task_registry_find(join(dir, 'src/lib/'), search_filters, dir);
		assert.deepEqual(entries, [
			{ name: 'src/lib/greet.task.ts', id: join(dir, 'src/lib/greet.task.ts') }
		]);
	});

	test('finds `.task.js` files', async () => {
		const dir = create_project(['src/lib/a.task.js', 'src/lib/b.task.ts']);
		const entries = await task_registry_find(join(dir, 'src'), search_filters, dir);
		assert.deepEqual(entries, [
			{ name: 'src/lib/a.task.js', id: join(dir, 'src/lib/a.task.js') },
			{ name: 'src/lib/b.task.ts', id: join(dir, 'src/lib/b.task.ts') }
		]);
	});

	test('skips tasks in node_modules and build output', async () => {
		const dir = create_project([
			'node_modules/@fuzdev/gro/dist/check.task.js',
			'dist/b.task.js',
			'build/c.task.js',
			'dist_server/d.task.js',
			'.svelte-kit/e.task.js',
			'a.task.ts'
		]);
		const entries = await task_registry_find(dir, search_filters, dir);
		assert.deepEqual(to_names(entries), ['a.task.ts']);
	});

	test('skips dot-directories inside the project', async () => {
		const dir = create_project(['src/.hidden/a.task.ts', 'src/b.task.ts']);
		const entries = await task_registry_find(join(dir, 'src'), search_filters, dir);
		assert.deepEqual(to_names(entries), ['src/b.task.ts']);
	});

	test('finds tasks in a project under a dot-directory', async () => {
		const dir = join(create_project(['.cache/project/src/lib/a.task.ts']), '.cache/project');
		const entries = await task_registry_find(join(dir, 'src'), search_filters, dir);
		assert.deepEqual(to_names(entries), ['src/lib/a.task.ts']);
	});

	test('filters relative to the root, whichever directory it searches', async () => {
		const dir = create_project(['src/lib/a.task.ts', 'src/lib/fixtures/b.task.ts']);
		const filters = [(p: string) => !p.startsWith('src/lib/fixtures')];
		for (const start of ['src', 'src/lib']) {
			const entries = await task_registry_find(join(dir, start), filters, dir);
			assert.deepEqual(to_names(entries), ['src/lib/a.task.ts']);
		}
	});
});

describe('task_registry_find_builtins', () => {
	test("finds Gro's builtins with their summaries, matching the generated ones", async () => {
		const entries = await task_registry_find_builtins(paths.lib);
		assert.deepEqual(
			Object.fromEntries(entries.map((e) => [e.name, e.summary])),
			GRO_BUILTIN_TASK_SUMMARIES
		);
		assert.deepEqual(
			entries.map((e) => e.id),
			entries.map((e) => to_builtin_task_id(e.name as keyof typeof GRO_BUILTIN_TASK_SUMMARIES))
		);
	});
});

describe('task_registry_render', () => {
	const entries: Array<TaskRegistryEntry> = [
		{ name: 'src/lib/greet.task.ts', id: '/p/src/lib/greet.task.ts' },
		{ name: 'src/lib/db/migrate.task.ts', id: '/p/src/lib/db/migrate.task.ts' },
		{ name: 'src/lib/db_migrate.task.js', id: '/p/src/lib/db_migrate.task.js' }
	];

	test('renders the ProjectTasks augmentation keyed by path', () => {
		const rendered = task_registry_render_project(entries, '/p/src', 'src/gro_tasks.gen.ts');
		assert.include(rendered, "import type * as src_lib_greet_task from './lib/greet.task.ts';");
		assert.include(
			rendered,
			"import type * as src_lib_db_migrate_task from './lib/db/migrate.task.ts';"
		);
		assert.include(
			rendered,
			"import type * as src_lib_db_migrate_task2 from './lib/db_migrate.task.js';"
		);
		assert.include(rendered, "declare module '@fuzdev/gro/task_registry.ts' {");
		assert.include(rendered, '\nexport {};', 'always a module, even with no tasks to import');
		assert.include(rendered, '\t\t"src/lib/greet.task.ts": typeof src_lib_greet_task;');
		assert.include(rendered, '\t\t"src/lib/db/migrate.task.ts": typeof src_lib_db_migrate_task;');
		assert.include(rendered, '\t\t"src/lib/db_migrate.task.js": typeof src_lib_db_migrate_task2;');
	});

	test('renders valid identifiers and quotes keys that need it', () => {
		const rendered = task_registry_render_project(
			[
				{ name: '2fa', id: '/p/src/lib/2fa.task.ts' },
				{ name: 'some-task', id: '/p/src/lib/some-task.task.ts' }
			],
			'/p/src',
			'src/gro_tasks.gen.ts'
		);
		assert.include(rendered, "import type * as _2fa_task from './lib/2fa.task.ts';");
		assert.include(rendered, '\t\t"2fa": typeof _2fa_task;');
		assert.include(rendered, "import type * as some_task_task from './lib/some-task.task.ts';");
		assert.include(rendered, '\t\t"some-task": typeof some_task_task;');
	});

	test('imports a task under a dot-directory by a path specifier, not a bare one', () => {
		const rendered = task_registry_render_project(
			[
				{ name: 'src/.hidden/a.task.ts', id: '/p/src/.hidden/a.task.ts' },
				{ name: 'b.task.ts', id: '/p/b.task.ts' }
			],
			'/p/src',
			'src/gro_tasks.gen.ts'
		);
		assert.include(rendered, "from './.hidden/a.task.ts';");
		assert.include(rendered, "from '../b.task.ts';");
	});

	test('renders the GroBuiltinTasks interface and the summaries', () => {
		const rendered = task_registry_render_builtins(
			[
				{ name: 'greet', id: '/p/src/lib/greet.task.ts', summary: "says 'hi'" },
				{ name: 'quiet', id: '/p/src/lib/quiet.task.ts', summary: '' }
			],
			'/p/src/lib',
			'x.gen.ts'
		);
		assert.include(rendered, "import type * as greet_task from './greet.task.ts';");
		assert.include(rendered, 'export interface GroBuiltinTasks {');
		assert.include(rendered, '\tgreet: typeof greet_task;');
		assert.include(
			rendered,
			'export const GRO_BUILTIN_TASK_SUMMARIES: Record<keyof GroBuiltinTasks, string> = {'
		);
		assert.include(rendered, `\tgreet: "says 'hi'",\n`);
		assert.include(rendered, '\tquiet: "",\n');
	});
});

describe('gen_task_registry', () => {
	test("generates the augmentation from the project's `src/` with specifiers relative to the genfile", async () => {
		const ctx = create_gen_context(
			{
				config: create_empty_gro_config(),
				log: create_mock_logger(),
				timings: create_mock_timings(),
				invoke_task: vi.fn()
			},
			join(paths.source, 'gro_tasks.gen.ts')
		);
		const rendered = await gen_task_registry.generate(ctx);
		assert.ok(typeof rendered === 'string');
		// Gro's own `src/` stands in for a project's
		assert.include(rendered, "import type * as src_lib_check_task from './lib/check.task.ts';");
		assert.include(rendered, '\t\t"src/lib/check.task.ts": typeof src_lib_check_task;');
		assert.notInclude(rendered, '\t\tcheck:');
	});

	test('regenerates when a task file changes', () => {
		const { inputs } = gen_task_registry;
		assert.ok(inputs);
		const matches = (id: string) => gen_inputs_match(inputs, id, '/p/');
		assert.ok(matches('/p/src/lib/a.task.ts'));
		assert.ok(matches('/p/src/lib/a.task.js'));
		assert.ok(!matches('/p/src/lib/a.ts'));
	});
});

describe('InvokeTask', () => {
	test('checks names and args at the type level', () => {
		// enforced by `gro typecheck` through the `@ts-expect-error` directives, never called
		const invoke_task_types = async (invoke_task: InvokeTask): Promise<void> => {
			await invoke_task('check');
			await invoke_task('check', { sync: false, workspace: true });
			await invoke_task('check', { 'no-sync': true });
			// @ts-expect-error the `gro/` prefix is removed
			await invoke_task('gro/check', { sync: false });
			await invoke_task('gen', { _: ['src/lib'], check: true });
			// @ts-expect-error unknown task name
			await invoke_task('chekc');
			// @ts-expect-error unknown arg
			await invoke_task('check', { synk: true });
			// @ts-expect-error wrong arg type
			await invoke_task('check', { sync: 'no' });
			await invoke_task('reinstall');
			await invoke_task('reinstall', {});
			// @ts-expect-error an arg for a task whose `Args` has no keys
			await invoke_task('reinstall', { x: 1 });
			// @ts-expect-error a path without a task suffix
			await invoke_task('src/lib/foo');
			// a task path not in the registry takes loose `Args`
			await invoke_task('src/lib/foo.task.ts', { anything: 1 });
			await invoke_task('./foo.task.js');
			const dynamic: string = 'some/path';
			await invoke_task(dynamic, { anything: 1 });
			for (const name of ['check', 'gen'] as const) await invoke_task(name, {});
		};
		assert.ok(invoke_task_types);
	});

	test('checks project tasks from the `ProjectTasks` augmentation', () => {
		// augmented by `fixtures/task_registry/project_tasks.ts`, never called
		const invoke_task_types = async (invoke_task: InvokeTask): Promise<void> => {
			await invoke_task('fixture/required_args.task.ts', { name: 'a' });
			await invoke_task('fixture/required_args.task.ts', { name: 'a', loud: true });
			// @ts-expect-error args are required when the schema has required fields
			await invoke_task('fixture/required_args.task.ts');
			// @ts-expect-error missing required arg
			await invoke_task('fixture/required_args.task.ts', { loud: true });
			// a leading `./` keys the registry like the path without it
			await invoke_task('./fixture/required_args.task.ts', { name: 'a' });
			// @ts-expect-error unknown arg through a leading `./`
			await invoke_task('./fixture/required_args.task.ts', { name: 'a', bad: 1 });
			// @ts-expect-error args are required through a leading `./`
			await invoke_task('./fixture/required_args.task.ts');
			// a union keeps each option's keys
			await invoke_task('fixture/union_args.task.ts', { a: 'x' });
			await invoke_task('fixture/union_args.task.ts', { b: 'y' });
			// @ts-expect-error an arg in no option of a union
			await invoke_task('fixture/union_args.task.ts', { c: 1 });
			// a module without an `Args` export accepts any args
			await invoke_task('fixture/no_args.task.ts', { anything: 1 });
			// a project task named like a builtin never shadows it
			await invoke_task('clean', { build_dev: true });
			// @ts-expect-error the project task's args don't apply to the builtin
			await invoke_task('clean', { deep: true });
			await invoke_task('fixture/clean.task.ts', { deep: true });
			// @ts-expect-error the builtin's args don't apply to the project task
			await invoke_task('fixture/clean.task.ts', { build_dev: true });
			// @ts-expect-error a project task's name without its path is no builtin
			await invoke_task('fixture/no_args');
		};
		assert.ok(invoke_task_types);
	});
});
