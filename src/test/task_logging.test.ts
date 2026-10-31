import { describe, test, assert, vi } from 'vitest';
import { stripVTControlCharacters } from 'node:util';
import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';

import { log_builtin_tasks, log_task_help, log_tasks } from '#lib/task_logging.ts';
import { GRO_BUILTIN_TASK_SUMMARIES } from '#lib/gro_builtin_tasks.ts';
import type { Task, TaskModuleMeta } from '#lib/task.ts';
import * as test_task_module from '#lib/test.task.ts';
import * as gen_task_module from '#lib/gen.task.ts';

import { create_mock_logger } from './test_helpers.ts';

const to_task_meta = (name: string, task: Task<any, any>): TaskModuleMeta => ({
	id: `/p/src/lib/${name}.task.ts`,
	name,
	mod: { task }
});

// the text `log_task_help` logs, unstyled, joined as the logger joins its arguments
const render_help = (
	task_meta: TaskModuleMeta
): { text: string; log: ReturnType<typeof create_mock_logger> } => {
	const log = create_mock_logger();
	log_task_help(log, task_meta);
	const info = vi.mocked(log.info);
	assert.strictEqual(info.mock.calls.length, 1);
	return { text: stripVTControlCharacters(info.mock.calls[0]!.join(' ')), log };
};

const escape_regexp = (v: string): string => v.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

// a row of the help table is its tokens in order, any padding between them
const assert_help_row = (
	text: string,
	name: string,
	type: string,
	default_text: string,
	description: string
): void => {
	const tokens = [name, type, default_text, description].map(escape_regexp);
	assert.match(text, new RegExp(`^${tokens.join('\\s+')}\\s*$`, 'm'));
};

describe('log_task_help', () => {
	test('prints a row per arg with its type, default, and description', () => {
		let made = 0;
		const Args = s.object({
			_: s.with_default_fn(meta(s.array(s.string), { description: 'the patterns' }), () => {
				made++;
				return ['.test.'];
			}),
			dir: s.with_default(meta(s.string, { description: 'the dir' }), 'src'),
			watch: s.with_default(meta(s.boolean, { description: 'dual of no-watch' }), true),
			'no-watch': s.with_default(meta(s.boolean, { description: 'opt out of watching' }), false),
			mode: s.with_default(
				meta(s.union([s.literal('a'), s.literal('b')]), { description: 'the mode' }),
				'a'
			),
			t: s.optional(meta(s.string, { description: 'a name pattern' })),
			bare: s.number
		});
		const task_meta = to_task_meta('fake', { summary: 'does a fake thing', Args, run: () => {} });
		const { text, log } = render_help(task_meta);
		assert.include(text, 'gro fake: does a fake thing');
		assert_help_row(text, '[...args]', 'Array<string>', "['.test.']", 'the patterns');
		assert_help_row(text, 'dir', 'string', "'src'", 'the dir');
		assert_help_row(text, 'no-watch', 'boolean', 'false', 'opt out of watching');
		assert_help_row(text, 'mode', '"a" | "b"', "'a'", 'the mode');
		assert_help_row(text, 't', 'string | undefined', 'undefined', 'a name pattern');
		assert_help_row(text, 'bare', 'number', 'undefined', '(no description available)');
		// a boolean with a `no-` dual shows only the dual, the flag a user passes to opt out
		assert.notMatch(text, /^watch\s/m);
		assert.notInclude(text, 'dual of no-watch');
		assert.strictEqual(made, 1, '`make` is called once per render');
		render_help(task_meta);
		assert.strictEqual(made, 2);
		assert.strictEqual(vi.mocked(log.error).mock.calls.length, 0);
	});

	test('makes a computed default under other layers', () => {
		const Args = s.object({
			paths: s.nullable(
				s.with_default_fn(meta(s.array(s.string), { description: 'the paths' }), () => ['a', 'b'])
			),
			piped: s.pipe(
				s.with_default_fn(s.number, () => 3),
				s.check((v: number) => v > 0)
			),
			lazied: s.lazy(() => s.with_default_fn(s.array(s.string), () => ['c']))
		});
		const { text } = render_help(to_task_meta('fake', { Args, run: () => {} }));
		assert_help_row(text, 'paths', 'Array<string> | null', "['a', 'b']", 'the paths');
		assert_help_row(text, 'piped', 'number', '3', '(no description available)');
		assert_help_row(text, 'lazied', 'Array<string>', "['c']", '(no description available)');
	});

	test('an `Args` with no entries says it has no args', () => {
		const { text } = render_help(to_task_meta('fake', { Args: s.object({}), run: () => {} }));
		assert.include(text, 'this task has no args');
	});

	test('a non-object `Args` logs an error and renders no rows', () => {
		const Args = s.union([s.object({ a: s.string }), s.object({ b: s.string })]);
		const { text, log } = render_help(to_task_meta('fake', { Args, run: () => {} }));
		assert.include(text, 'this task has no args');
		assert.notMatch(text, /^a\s/m);
		const error = vi.mocked(log.error);
		assert.strictEqual(error.mock.calls.length, 1);
		assert.include(
			String(error.mock.calls[0]![0]),
			'Expected Args for task "fake" to be an object schema but got union'
		);
	});

	test('a project task path shows the `gro task` command', () => {
		const task_meta = { ...to_task_meta('x', { summary: 'a path task', run: () => {} }) };
		task_meta.name = 'src/lib/x.task.ts';
		const { text } = render_help(task_meta);
		assert.include(text, 'gro task src/lib/x.task.ts: a path task');
	});

	test('a task without `Args` renders no table', () => {
		const { text } = render_help(to_task_meta('fake', { run: () => {} }));
		assert.include(text, 'gro fake: (no summary available)');
		assert.notInclude(text, 'this task has no args');
	});
});

describe('log_task_help for builtins', () => {
	// computed defaults once printed as `undefined`
	test('`gro test` shows the default made for `_`', () => {
		const { text } = render_help(to_task_meta('test', test_task_module.task));
		assert_help_row(
			text,
			'[...args]',
			'Array<string>',
			"['.test.']",
			'file patterns to filter tests'
		);
	});

	test('`gro gen` shows the defaults made for `_` and `root_dirs`', () => {
		const { text, log } = render_help(to_task_meta('gen', gen_task_module.task));
		assert_help_row(text, '[...args]', 'Array<string>', "['src']", 'input paths to generate');
		assert_help_row(
			text,
			'root_dirs',
			'Array<string>',
			`['${process.cwd()}']`,
			'root directories to resolve input paths against'
		);
		assert.strictEqual(vi.mocked(log.error).mock.calls.length, 0);
	});
});

describe('log_tasks', () => {
	const render = (modules: Array<TaskModuleMeta>): string => {
		const log = create_mock_logger();
		log_tasks(log, modules, './src');
		const info = vi.mocked(log.info);
		assert.strictEqual(info.mock.calls.length, 1);
		return stripVTControlCharacters(info.mock.calls[0]!.join(' '));
	};

	test('lists each task by path with its summary and how to run one', () => {
		const text = render([
			{ ...to_task_meta('a', { summary: 'does a', run: () => {} }), name: 'src/a.task.ts' },
			{ ...to_task_meta('b', { run: () => {} }), name: 'src/lib/b.task.ts' }
		]);
		assert.include(text, '2 tasks in ./src:');
		assert.match(text, /^src\/a\.task\.ts\s+does a$/m);
		assert.match(text, /^src\/lib\/b\.task\.ts\s*$/m);
		assert.include(text, 'Run a task: gro task [path]');
		assert.include(text, 'View help:  gro task [path] --help');
	});

	test('says when there are none', () => {
		assert.include(render([]), 'No tasks found in ./src.');
	});
});

describe('log_builtin_tasks', () => {
	test('lists the builtins with their summaries and the usage, importing no task module', () => {
		const log = create_mock_logger();
		log_builtin_tasks(log);
		const text = stripVTControlCharacters(vi.mocked(log.info).mock.calls[0]!.join(' '));
		const names = Object.keys(GRO_BUILTIN_TASK_SUMMARIES);
		assert.include(text, `${names.length} builtin tasks:`);
		assert.match(text, /^check\s+check that everything is ready to commit$/m);
		assert.match(text, /^test\s+run tests with vitest$/m);
		assert.include(text, 'Run a builtin:        gro [name]');
		assert.include(text, 'Run a project task:   gro task [path/to/file.task.ts]');
		assert.include(text, 'List project tasks:   gro task [dir], defaults to src/');
		assert.notInclude(text, 'resolve');
	});
});
