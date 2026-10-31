import { describe, test, assert, vi, afterEach, beforeEach } from 'vitest';
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { Logger } from '@fuzdev/fuz_util/log.ts';
import { dirname, join, relative } from 'node:path';

import { invoke_cli, resolve_task } from '#lib/invoke_task.ts';
import { run_task } from '#lib/run_task.ts';
import { log_builtin_tasks, log_error_reasons, log_tasks } from '#lib/task_logging.ts';
import { create_empty_gro_config, type GroConfig } from '#lib/gro_config.ts';
import { SilentError, to_builtin_task_id } from '#lib/task.ts';
import { paths } from '#lib/paths.ts';
import { GRO_BUILTIN_TASK_SUMMARIES } from '#lib/gro_builtin_tasks.ts';
import { to_task_args as parse_cli_task_args, type CliTaskArgs } from '#lib/args.ts';

vi.mock('#lib/run_task.ts', () => ({
	run_task: vi.fn(async () => ({ ok: true }))
}));

vi.mock('#lib/task_logging.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/task_logging.ts')>()),
	log_builtin_tasks: vi.fn(),
	log_tasks: vi.fn(),
	log_error_reasons: vi.fn()
}));

let dir: string;

beforeEach(() => {
	dir = mkdtempSync(join(tmpdir(), 'gro_invoke_task_'));
});

afterEach(() => {
	rmSync(dir, { recursive: true, force: true });
	vi.clearAllMocks();
});

const write_tasks = (files: Array<string>): void => {
	for (const file of files) {
		const id = join(dir, file);
		mkdirSync(dirname(id), { recursive: true });
		writeFileSync(id, `export const task = {summary: 'does ${file}', run: () => {}};\n`);
	}
};

const config: GroConfig = create_empty_gro_config();

const to_task_args = (
	task_name: string,
	task_path: string | null = null,
	args: CliTaskArgs['args'] = {}
): CliTaskArgs => ({ task_name, task_path, args });

const error_text = (): string =>
	vi
		.mocked(log_error_reasons)
		.mock.calls.flatMap((c) => c[1])
		.join('\n');

describe('resolve_task', () => {
	test('a bare name is a builtin', async () => {
		const resolved = await resolve_task('check', dir);
		assert.deepEqual(resolved, { ok: true, id: to_builtin_task_id('check'), name: 'check' });
	});

	test('a bare name is never a project task, even when one exists by that name', async () => {
		write_tasks(['src/lib/check.task.ts', 'src/lib/greet.task.ts', 'greet.task.ts']);
		const resolved = await resolve_task('check', dir);
		assert.ok(resolved.ok);
		assert.strictEqual(resolved.id, to_builtin_task_id('check'));
		assert.ok(!(await resolve_task('greet', dir)).ok);
	});

	test('an unknown name lists the builtins and hints the path, saying the file is missing', async () => {
		const resolved = await resolve_task('greet', dir);
		assert.ok(!resolved.ok);
		assert.strictEqual(resolved.reasons[0], 'No builtin `greet`.');
		assert.include(resolved.reasons[1], 'Builtins: build, changeset, check,');
		assert.strictEqual(
			resolved.reasons[2],
			'Project tasks run by path, like `gro task src/lib/greet.task.ts` (no file there).'
		);
	});

	test('an unknown name says when the hinted file exists', async () => {
		write_tasks(['src/lib/greet.task.ts']);
		const resolved = await resolve_task('greet', dir);
		assert.ok(!resolved.ok);
		assert.strictEqual(
			resolved.reasons[2],
			'`src/lib/greet.task.ts` exists, run it by path: `gro task src/lib/greet.task.ts`'
		);
	});

	test('a name that could not be a file stem gets no path hint', async () => {
		const resolved = await resolve_task('task.ts', dir);
		assert.deepEqual(resolved, {
			ok: false,
			reasons: [
				'No builtin `task.ts`.',
				`Builtins: ${Object.keys(GRO_BUILTIN_TASK_SUMMARIES).join(', ')}`
			]
		});
	});

	test('the `gro/` prefix errors naming the plain builtin', async () => {
		const resolved = await resolve_task('gro/check', dir);
		assert.deepEqual(resolved, {
			ok: false,
			reasons: [
				'The `gro/` prefix is removed, builtins are plain names: use `check`, as in `gro check`.'
			]
		});
		const unknown = await resolve_task('gro/greet', dir);
		assert.deepEqual(unknown, {
			ok: false,
			reasons: ["The `gro/` prefix is removed, and there's no builtin `greet`."]
		});
		assert.deepEqual(await resolve_task('gro/', dir), {
			ok: false,
			reasons: ['The `gro/` prefix is removed, builtins are plain names: run `gro` to list them.']
		});
	});

	test('a path needs a task suffix, with no probing, hinting what exists', async () => {
		write_tasks(['src/lib/greet.task.ts', 'src/lib/hi.task.js', 'src/lib/hi/a.task.ts']);
		const not_a_task_path = (input: string) =>
			`\`${input}\` is not a task path: task paths end in \`.task.ts\` or \`.task.js\`.`;
		assert.deepEqual(await resolve_task('src/lib/greet', dir), {
			ok: false,
			reasons: [
				not_a_task_path('src/lib/greet'),
				'Run the task file with `gro task src/lib/greet.task.ts`.'
			]
		});
		assert.deepEqual(await resolve_task('src/lib/hi', dir), {
			ok: false,
			reasons: [
				not_a_task_path('src/lib/hi'),
				'Run the task file with `gro task src/lib/hi.task.js`. ' +
					"List the directory's tasks with `gro task src/lib/hi`."
			]
		});
		write_tasks(['src/lib/ok.task.ts']);
		assert.deepEqual(await resolve_task('src/lib/ok.task', dir), {
			ok: false,
			reasons: [
				not_a_task_path('src/lib/ok.task'),
				'Run the task file with `gro task src/lib/ok.task.ts`.'
			]
		});
		assert.deepEqual(await resolve_task('src/lib/no.task', dir), {
			ok: false,
			reasons: [
				not_a_task_path('src/lib/no.task'),
				'Nothing is at `src/lib/no.task`, `src/lib/no.task.ts`, or `src/lib/no.task.js`.'
			]
		});
		assert.deepEqual(await resolve_task('src/lib/hi/', dir), {
			ok: false,
			reasons: [
				not_a_task_path('src/lib/hi/'),
				"List the directory's tasks with `gro task src/lib/hi/`."
			]
		});
		assert.deepEqual(await resolve_task('src/lib/missing', dir), {
			ok: false,
			reasons: [
				not_a_task_path('src/lib/missing'),
				'Nothing is at `src/lib/missing`, `src/lib/missing.task.ts`, or `src/lib/missing.task.js`.'
			]
		});
		// a file without a task suffix needs no more than the suffix rule
		writeFileSync(join(dir, 'src/lib/plain.ts'), '');
		assert.deepEqual(await resolve_task('src/lib/plain.ts', dir), {
			ok: false,
			reasons: [not_a_task_path('src/lib/plain.ts')]
		});
	});

	test('a task path naming a directory is not a task file', async () => {
		mkdirSync(join(dir, 'src/lib/x.task.ts'), { recursive: true });
		assert.deepEqual(await resolve_task('src/lib/x.task.ts', dir), {
			ok: false,
			reasons: ["`src/lib/x.task.ts` is not a task file: it's a directory or a special file."]
		});
	});

	test('a path is relative to the root, `./` optional, absolute accepted, normalized', async () => {
		write_tasks(['src/lib/greet.task.ts', 'top.task.js']);
		const id = join(dir, 'src/lib/greet.task.ts');
		const expected = { ok: true as const, id, name: 'src/lib/greet.task.ts' };
		assert.deepEqual(await resolve_task('src/lib/greet.task.ts', dir), expected);
		assert.deepEqual(await resolve_task('./src/lib/greet.task.ts', dir), expected);
		assert.deepEqual(await resolve_task('src/lib/../lib/greet.task.ts', dir), expected);
		assert.deepEqual(await resolve_task(id, dir), expected);
		assert.deepEqual(await resolve_task('top.task.js', dir), {
			ok: true,
			id: join(dir, 'top.task.js'),
			name: 'top.task.js'
		});
	});

	test('a missing task file errors with its normalized path', async () => {
		const resolved = await resolve_task('./src/lib/missing.task.ts', dir);
		assert.deepEqual(resolved, {
			ok: false,
			reasons: ['No task file at `src/lib/missing.task.ts`.']
		});
	});
});

describe('invoke_cli', () => {
	test('`gro` lists the builtins', async () => {
		await invoke_cli(to_task_args(''), config);
		assert.strictEqual(vi.mocked(log_builtin_tasks).mock.calls.length, 1);
		assert.strictEqual(vi.mocked(run_task).mock.calls.length, 0);
	});

	test('`gro <name>` runs the builtin', async () => {
		await invoke_cli(to_task_args('typecheck'), config);
		assert.strictEqual(vi.mocked(run_task).mock.calls.length, 1);
		assert.strictEqual(vi.mocked(run_task).mock.calls[0]![0].name, 'typecheck');
	});

	test('`gro task <path>` runs the task file, named by its root-relative path', async () => {
		write_tasks(['src/lib/greet.task.ts']);
		const id = join(dir, 'src/lib/greet.task.ts');
		await invoke_cli(to_task_args('task', id, { a: 1 }), config);
		assert.strictEqual(vi.mocked(run_task).mock.calls.length, 1);
		const [task_meta, args] = vi.mocked(run_task).mock.calls[0]!;
		assert.strictEqual(task_meta.id, id);
		assert.strictEqual(task_meta.name, relative(process.cwd(), id));
		assert.strictEqual(task_meta.mod.task.summary, 'does src/lib/greet.task.ts');
		assert.deepEqual(args, { a: 1 });
	});

	test('`gro task <dir>` lists the tasks under it, without running them', async () => {
		write_tasks(['src/lib/b.task.ts', 'src/a.task.js', 'top.task.ts']);
		await invoke_cli(to_task_args('task', join(dir, 'src')), config);
		assert.strictEqual(vi.mocked(run_task).mock.calls.length, 0);
		const calls = vi.mocked(log_tasks).mock.calls;
		assert.strictEqual(calls.length, 1);
		assert.deepEqual(
			calls[0]![1].map((m) => m.name),
			['src/a.task.js', 'src/lib/b.task.ts'].map((p) => relative(process.cwd(), join(dir, p)))
		);
	});

	test('`gro task` with no path lists `src/`', async () => {
		await invoke_cli(to_task_args('task'), { ...config, search_filters: [() => false] });
		const calls = vi.mocked(log_tasks).mock.calls;
		assert.strictEqual(calls.length, 1);
		assert.include(calls[0]![2], 'src');
	});

	test('`gro task <path>` that is neither a directory nor a task file errors, naming the task file', async () => {
		write_tasks(['src/lib/greet.task.ts']);
		const path = join(dir, 'src/lib/greet');
		await assert_silent(invoke_cli(to_task_args('task', path), config));
		assert.strictEqual(
			error_text(),
			`\`${path}\` is not a directory or a task file: task paths end in \`.task.ts\` or \`.task.js\`.\n` +
				`Run the task file with \`gro task ${path}.task.ts\`.`
		);
		vi.clearAllMocks();
		const missing = join(dir, 'src/lib/missing');
		await assert_silent(invoke_cli(to_task_args('task', missing), config));
		assert.include(
			error_text(),
			`Nothing is at \`${missing}\`, \`${missing}.task.ts\`, or \`${missing}.task.js\`.`
		);
	});

	test('`gro task <dir>` with no tasks beside a task file of its name hints running it', async () => {
		write_tasks(['src/fixtures/update.task.ts']);
		mkdirSync(join(dir, 'src/fixtures/update'));
		const info = vi.spyOn(Logger.prototype, 'info');
		try {
			await in_dir(dir, () => invoke_cli(to_task_args('task', 'src/fixtures/update'), config));
			assert.deepEqual(info.mock.calls, [
				['Run the task file with `gro task src/fixtures/update.task.ts`.']
			]);
		} finally {
			info.mockRestore();
		}
		assert.strictEqual(vi.mocked(log_tasks).mock.calls.length, 1);
	});

	test('`gro task <builtin>` says how to run the builtin', async () => {
		await assert_silent(invoke_cli(to_task_args('task', 'check'), config));
		assert.strictEqual(
			error_text(),
			'`check` is not a directory or a task file: task paths end in `.task.ts` or `.task.js`.\n' +
				'`check` is a builtin: run it with `gro check`.'
		);
	});

	test('`gro task <dir>` lists the tasks that load, then reports each that fails', async () => {
		write_tasks(['src/a.task.ts', 'src/c.task.ts']);
		writeFileSync(join(dir, 'src/b.task.ts'), 'export const task = {};\n');
		writeFileSync(join(dir, 'src/d.task.ts'), 'throw new Error("broken");\n');
		await assert_silent(invoke_cli(to_task_args('task', join(dir, 'src')), config));
		const calls = vi.mocked(log_tasks).mock.calls;
		assert.strictEqual(calls.length, 1);
		assert.deepEqual(
			calls[0]![1].map((m) => m.name),
			['src/a.task.ts', 'src/c.task.ts'].map((p) => relative(process.cwd(), join(dir, p)))
		);
		const reasons = vi.mocked(log_error_reasons).mock.calls.flatMap((c) => c[1]);
		assert.strictEqual(reasons.length, 2);
		assert.include(reasons[0], 'b.task.ts');
		assert.include(reasons[1], 'd.task.ts');
	});

	test('`gro task <dir>` with only broken tasks reports them without an empty listing', async () => {
		mkdirSync(join(dir, 'src'));
		writeFileSync(join(dir, 'src/b.task.ts'), 'export const task = {};\n');
		await assert_silent(invoke_cli(to_task_args('task', join(dir, 'src')), config));
		assert.strictEqual(vi.mocked(log_tasks).mock.calls.length, 0);
		assert.include(error_text(), 'b.task.ts');
	});

	test('`gro task <dir>` finds tasks in a project under a dot-directory', async () => {
		const project = join(dir, '.cache/project');
		write_tasks(['.cache/project/src/lib/a.task.ts', '.cache/project/src/.hidden/b.task.ts']);
		const cwd = process.cwd();
		try {
			process.chdir(project); // the CLI's cwd is the project root
			await invoke_cli(to_task_args('task', 'src'), config);
		} finally {
			process.chdir(cwd);
		}
		const calls = vi.mocked(log_tasks).mock.calls;
		assert.deepEqual(
			calls[0]![1].map((m) => m.name),
			['src/lib/a.task.ts']
		);
	});

	test('a path without `gro task` errors with the hint', async () => {
		await assert_silent(invoke_cli(to_task_args('src/lib/greet.task.ts'), config));
		assert.strictEqual(
			error_text(),
			'`src/lib/greet.task.ts` is a path: run a task file with `gro task src/lib/greet.task.ts`.'
		);
		vi.clearAllMocks();
		await assert_silent(invoke_cli(to_task_args('greet.task.ts'), config));
		assert.include(error_text(), 'run a task file with `gro task greet.task.ts`');
		vi.clearAllMocks();
		await assert_silent(invoke_cli(to_task_args('src/lib'), config));
		assert.strictEqual(
			error_text(),
			"`src/lib` is a path, not a builtin name. List the directory's tasks with `gro task src/lib`."
		);
		vi.clearAllMocks();
		write_tasks(['src/fixtures/update.task.ts']);
		await in_dir(dir, () => assert_silent(invoke_cli(to_task_args('src/fixtures/update'), config)));
		assert.strictEqual(
			error_text(),
			'`src/fixtures/update` is a path, not a builtin name. ' +
				'Run the task file with `gro task src/fixtures/update.task.ts`.'
		);
		vi.clearAllMocks();
		await in_dir(dir, () => assert_silent(invoke_cli(to_task_args('src/fixtures/none'), config)));
		assert.strictEqual(
			error_text(),
			'`src/fixtures/none` is a path, not a builtin name. ' +
				'Nothing is at `src/fixtures/none`, `src/fixtures/none.task.ts`, or `src/fixtures/none.task.js`.'
		);
		assert.strictEqual(vi.mocked(log_tasks).mock.calls.length, 0);
		assert.strictEqual(vi.mocked(run_task).mock.calls.length, 0);
	});

	test('`gro gro/<name>` errors naming the plain builtin', async () => {
		await assert_silent(invoke_cli(to_task_args('gro/check'), config));
		assert.include(error_text(), 'use `check`, as in `gro check`');
	});

	test('`gro <unknown>` errors with no listing fallthrough', async () => {
		await assert_silent(invoke_cli(to_task_args('greet'), config));
		assert.include(error_text(), 'No builtin `greet`.');
		assert.strictEqual(vi.mocked(log_tasks).mock.calls.length, 0);
		assert.strictEqual(vi.mocked(log_builtin_tasks).mock.calls.length, 0);
	});

	test('`gro task` is the verb, never a builtin name', async () => {
		assert.ok(!(await resolve_task('task', paths.root)).ok);
		write_tasks(['src/lib/a.task.ts']);
		await in_dir(dir, async () => {
			await invoke_cli(parse_cli_task_args(['node', 'gro', 'task']), config);
			await invoke_cli(
				parse_cli_task_args(['node', 'gro', 'task', 'src/lib/a.task.ts', '--x']),
				config
			);
		});
		const listed = vi.mocked(log_tasks).mock.calls;
		assert.strictEqual(listed.length, 1);
		assert.deepEqual(
			listed[0]![1].map((m) => m.name),
			['src/lib/a.task.ts']
		);
		const ran = vi.mocked(run_task).mock.calls;
		assert.strictEqual(ran.length, 1);
		assert.strictEqual(ran[0]![0].name, 'src/lib/a.task.ts');
		assert.deepEqual(ran[0]![1], { x: true });
	});

	test('`gro task` in a project with no `src/` says so, blaming no path', async () => {
		await in_dir(dir, () => assert_silent(invoke_cli(to_task_args('task'), config)));
		assert.strictEqual(
			error_text(),
			"`gro task` lists the tasks under `src/`, and there's no `src/` directory here: " +
				"run a task file with `gro task <path>` or list a directory's with `gro task <dir>`."
		);
		assert.strictEqual(vi.mocked(log_tasks).mock.calls.length, 0);
	});
});

/**
 * Runs `fn` with `cwd` as the process cwd, the CLI's project root.
 */
const in_dir = async (cwd: string, fn: () => Promise<unknown>): Promise<void> => {
	const original = process.cwd();
	try {
		process.chdir(cwd);
		await fn();
	} finally {
		process.chdir(original);
	}
};

const assert_silent = async (promise: Promise<unknown>): Promise<void> => {
	try {
		await promise;
	} catch (error) {
		assert.instanceOf(error, SilentError);
		return;
	}
	assert.fail('expected a SilentError');
};
