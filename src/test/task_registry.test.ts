import { describe, test, assert, afterEach, vi } from 'vitest';
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';

import {
	task_registry_find,
	task_registry_render_builtins,
	task_registry_render_project,
	gen_task_registry,
	type TaskRegistryEntry
} from '#lib/task_registry.ts';
import { create_gen_context } from '#lib/gen.ts';
import { create_empty_gro_config } from '#lib/gro_config.ts';
import { GRO_DIST_DIR } from '#lib/paths.ts';
import type { InvokeTask } from '#lib/task.ts';

import { create_mock_filer, create_mock_logger, create_mock_timings } from './test_helpers.ts';

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

	test('names tasks relative to their root dir, sorted', async () => {
		const dir = create_project([
			'src/lib/greet.task.ts',
			'src/lib/db/migrate.task.ts',
			'src/lib/not_a_task.ts',
			'src/test/fixtures/update.task.ts'
		]);
		const entries = await task_registry_find([join(dir, 'src/lib'), dir], search_filters);
		assert.deepEqual(to_names(entries), ['db/migrate', 'greet', 'src/test/fixtures/update']);
	});

	test('a file found under an earlier root dir is not renamed by a later one', async () => {
		const dir = create_project(['src/lib/greet.task.ts']);
		const entries = await task_registry_find([join(dir, 'src/lib'), dir], search_filters);
		assert.deepEqual(entries, [{ name: 'greet', id: join(dir, 'src/lib/greet.task.ts') }]);
	});

	test('a name from an earlier root dir shadows the same name in a later one', async () => {
		const dir = create_project(['src/lib/greet.task.ts', 'greet.task.ts']);
		const entries = await task_registry_find([join(dir, 'src/lib'), dir], search_filters);
		assert.deepEqual(entries, [{ name: 'greet', id: join(dir, 'src/lib/greet.task.ts') }]);
	});

	test('finds `.task.js` files', async () => {
		const dir = create_project(['src/lib/a.task.js', 'src/lib/b.task.ts']);
		const entries = await task_registry_find([join(dir, 'src/lib')], search_filters);
		assert.deepEqual(entries, [
			{ name: 'a', id: join(dir, 'src/lib/a.task.js') },
			{ name: 'b', id: join(dir, 'src/lib/b.task.ts') }
		]);
	});

	test('skips tasks in node_modules', async () => {
		const dir = create_project(['node_modules/@fuzdev/gro/dist/check.task.js', 'a.task.ts']);
		const entries = await task_registry_find([dir], search_filters);
		assert.deepEqual(to_names(entries), ['a']);
	});

	test("skips Gro's dist root dir, which config normalizes without a trailing slash", async () => {
		const entries = await task_registry_find([GRO_DIST_DIR.slice(0, -1)], search_filters);
		assert.deepEqual(entries, []);
	});
});

describe('task_registry_render', () => {
	const entries: Array<TaskRegistryEntry> = [
		{ name: 'greet', id: '/p/src/lib/greet.task.ts' },
		{ name: 'db/migrate', id: '/p/src/lib/db/migrate.task.ts' },
		{ name: 'db_migrate', id: '/p/src/lib/db_migrate.task.ts' }
	];

	test('renders the ProjectTasks augmentation', () => {
		const rendered = task_registry_render_project(entries, '/p/src', 'src/gro_tasks.gen.ts');
		assert.include(rendered, "import type * as greet_task from './lib/greet.task.ts';");
		assert.include(rendered, "import type * as db_migrate_task from './lib/db/migrate.task.ts';");
		assert.include(rendered, "import type * as db_migrate_task2 from './lib/db_migrate.task.ts';");
		assert.include(rendered, "declare module '@fuzdev/gro/task_registry.ts' {");
		assert.include(rendered, '\t\tgreet: typeof greet_task;');
		assert.include(rendered, '\t\t"db/migrate": typeof db_migrate_task;');
		assert.include(rendered, '\t\tdb_migrate: typeof db_migrate_task2;');
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

	test('renders the GroBuiltinTasks interface', () => {
		const rendered = task_registry_render_builtins(entries, '/p/src/lib', 'x.gen.ts');
		assert.include(rendered, "import type * as greet_task from './greet.task.ts';");
		assert.include(rendered, 'export interface GroBuiltinTasks {');
		assert.include(rendered, '\tgreet: typeof greet_task;');
	});
});

describe('gen_task_registry', () => {
	test('generates the augmentation from the config with specifiers relative to the genfile', async () => {
		const dir = create_project([
			'src/lib/greet.task.ts',
			'src/test/fixtures/update.task.ts',
			'node_modules/@fuzdev/gro/dist/check.task.js'
		]);
		const config = create_empty_gro_config();
		config.task_root_dirs = [join(dir, 'src/lib'), dir];
		const ctx = create_gen_context(
			{
				config,
				filer: create_mock_filer(),
				log: create_mock_logger(),
				timings: create_mock_timings(),
				invoke_task: vi.fn()
			},
			join(dir, 'src/gro_tasks.gen.ts')
		);
		const rendered = await gen_task_registry.generate(ctx);
		assert.ok(typeof rendered === 'string');
		assert.include(rendered, "import type * as greet_task from './lib/greet.task.ts';");
		assert.include(
			rendered,
			"import type * as src_test_fixtures_update_task from './test/fixtures/update.task.ts';"
		);
		assert.include(rendered, '\t\tgreet: typeof greet_task;');
		assert.include(
			rendered,
			'\t\t"src/test/fixtures/update": typeof src_test_fixtures_update_task;'
		);
		assert.notInclude(rendered, 'check');
	});

	test('regenerates when a task file changes', () => {
		const { dependencies } = gen_task_registry;
		assert.ok(dependencies && typeof dependencies === 'object' && dependencies.patterns);
		const matches = (id: string) => dependencies.patterns!.some((p) => p.test(id));
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
			await invoke_task('gro/check', { sync: false });
			await invoke_task('gen', { _: ['src/lib'], check: true });
			// @ts-expect-error unknown task name
			await invoke_task('chekc');
			// @ts-expect-error unknown arg
			await invoke_task('check', { synk: true });
			// @ts-expect-error wrong arg type
			await invoke_task('check', { sync: 'no' });
			// @ts-expect-error unknown arg for a `gro/` name
			await invoke_task('gro/check', { synk: true });
			const dynamic: string = 'some/path';
			await invoke_task(dynamic, { anything: 1 });
			for (const name of ['check', 'gen'] as const) await invoke_task(name, {});
		};
		assert.ok(invoke_task_types);
	});

	test('checks project tasks from the `ProjectTasks` augmentation', () => {
		// augmented by `fixtures/task_registry/project_tasks.ts`, never called
		const invoke_task_types = async (invoke_task: InvokeTask): Promise<void> => {
			await invoke_task('fixture/required_args', { name: 'a' });
			await invoke_task('fixture/required_args', { name: 'a', loud: true });
			// @ts-expect-error args are required when the schema has required fields
			await invoke_task('fixture/required_args');
			// @ts-expect-error missing required arg
			await invoke_task('fixture/required_args', { loud: true });
			// a module without an `Args` export accepts any args
			await invoke_task('fixture/no_args', { anything: 1 });
			// a project task shadows the builtin with the same name
			await invoke_task('clean', { deep: true });
			// @ts-expect-error the builtin's args don't apply to the shadowing task
			await invoke_task('clean', { build_dev: true });
			// the `gro/` name still reaches the builtin
			await invoke_task('gro/clean', { build_dev: true });
			// @ts-expect-error the shadowing task's args don't apply to the builtin
			await invoke_task('gro/clean', { deep: true });
		};
		assert.ok(invoke_task_types);
	});
});
