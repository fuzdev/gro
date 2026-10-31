import { args_parse, args_serialize, argv_parse } from '@fuzdev/fuz_util/args.ts';
import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';
import { describe, test, expect } from 'vitest';
import { resolve } from 'node:path';

import {
	has_forwarded_flag,
	merge_forwarded_args,
	to_forwarded_args,
	to_forwarded_args_by_command,
	to_forwarded_argv,
	to_forwarded_flag_value,
	to_implicit_forwarded_argv,
	to_raw_rest_args,
	to_task_args
} from '#lib/args.ts';
import {
	TaskError,
	to_removed_builtin_prefix_message,
	to_task_path_without_verb_message
} from '#lib/task.ts';

describe('to_forwarded_args_by_command', () => {
	test('parses only gro sections, keyed by the task', () => {
		const raw_rest_args = to_raw_rest_args([
			'gro',
			'taskname',
			'a',
			'--d',
			'--',
			'--',
			'eslint',
			'a',
			'--b',
			'c',
			'--',
			'gro',
			'a',
			'--a',
			'--',
			'gro',
			'b',
			'-t2',
			't2a',
			'--t2',
			't2b',
			'--t222',
			'2',
			'--',
			'groc',
			'--m',
			'--',
			'gro',
			'd',
			'-b',
			'a',
			'--c',
			'4',
			'--',
			'gro',
			'd',
			'-b',
			'a',
			'--c',
			'5'
		]);
		expect(to_forwarded_args_by_command(raw_rest_args)).toEqual({
			'gro a': { a: true },
			'gro b': { '2': 't2a', t: true, t2: 't2b', t222: 2 },
			'gro d': { b: 'a', c: 5 }
		});
		expect(to_forwarded_args('gro b', raw_rest_args)).toEqual({
			'2': 't2a',
			t: true,
			t2: 't2b',
			t222: 2
		});
	});

	test('skips sections without a command and tool sections', () => {
		const raw_rest_args = to_raw_rest_args('gro test -- --flag -- eslint --fix'.split(' '));
		expect(to_forwarded_args_by_command(raw_rest_args)).toEqual({});
	});

	test('throws when gro without taskname', () => {
		const raw_rest_args = to_raw_rest_args('gro test -- gro'.split(' '));
		expect(() => to_forwarded_args_by_command(raw_rest_args)).toThrow(TaskError);
		expect(() => to_forwarded_args_by_command(raw_rest_args)).toThrow(
			'Malformed args following a `--`. Expected gro taskname'
		);
	});

	test('keys `gro task` by the normalized path after it', () => {
		const raw_rest_args = to_raw_rest_args(
			'gro check -- gro task ./src/lib/a.task.ts --b -- gro task src/c.task.ts --d'.split(' ')
		);
		expect(to_forwarded_args_by_command(raw_rest_args)).toEqual({
			'gro task src/lib/a.task.ts': { b: true },
			'gro task src/c.task.ts': { d: true }
		});
		const absolute = to_raw_rest_args(['gro', 'check', '--', 'gro', 'task', resolve('x.task.ts')]);
		expect(Object.keys(to_forwarded_args_by_command(absolute))).toEqual(['gro task x.task.ts']);
	});

	test('throws when `gro task` has no path', () => {
		const raw_rest_args = to_raw_rest_args('gro check -- gro task --b'.split(' '));
		expect(() => to_forwarded_args_by_command(raw_rest_args)).toThrow(TaskError);
		expect(() => to_forwarded_args_by_command(raw_rest_args)).toThrow(
			'Malformed args following a `--`. Expected a path after `gro task`'
		);
	});

	test('a path without `gro task` throws the CLI hint', () => {
		for (const [input, message] of [
			['src/lib/x.task.ts', 'run a task file with `gro task src/lib/x.task.ts`'],
			['src/lib', "list a directory's tasks with `gro task src/lib`"]
		] as const) {
			const raw_rest_args = to_raw_rest_args(['gro', 'check', '--', 'gro', input, '--a']);
			expect(() => to_forwarded_args_by_command(raw_rest_args)).toThrow(TaskError);
			expect(() => to_forwarded_args_by_command(raw_rest_args)).toThrow(
				to_task_path_without_verb_message(input)
			);
			expect(to_task_path_without_verb_message(input)).toContain(message);
		}
	});

	test('the removed `gro/` prefix throws the CLI hint', () => {
		const raw_rest_args = to_raw_rest_args('gro check -- gro gro/test --a'.split(' '));
		expect(() => to_forwarded_args_by_command(raw_rest_args)).toThrow(TaskError);
		expect(() => to_forwarded_args_by_command(raw_rest_args)).toThrow(
			to_removed_builtin_prefix_message('gro/test')
		);
		expect(to_removed_builtin_prefix_message('gro/test')).toContain('use `test`');
	});

	test('a forwarded flag replacing the positional args is a usage error', () => {
		const raw_rest_args = to_raw_rest_args('gro check -- gro test --no-_'.split(' '));
		expect(() => to_forwarded_args_by_command(raw_rest_args)).toThrow(TaskError);
		// a tool's section is never parsed
		expect(
			to_forwarded_argv('vitest', to_raw_rest_args('gro test -- vitest --no-_'.split(' ')))
		).toEqual(['--no-_']);
	});

	test('throws when gro with only flags', () => {
		const raw_rest_args = to_raw_rest_args('gro test -- gro --flag'.split(' '));
		expect(() => to_forwarded_args_by_command(raw_rest_args)).toThrow(
			'Malformed args following a `--`. Expected gro taskname'
		);
	});
});

describe('to_forwarded_argv', () => {
	const forward = (...section: Array<string>): Array<string> =>
		to_forwarded_argv('tool', to_raw_rest_args(['node', 'gro', 'x', '--', 'tool', ...section]));

	test('passes the text verbatim, unlike a parse and serialize', () => {
		const cases: Array<Array<string>> = [
			['--no-cache'],
			['--max-warnings', '-1'],
			['--grep', '-foo'],
			['--port', '0123'],
			['--version', '1.10'],
			['--id', '1e3'],
			['--x='],
			['--x', ''],
			['a', '--b', 'c', '-def', '--no-g=1']
		];
		for (const section of cases) {
			expect(forward(...section)).toEqual(section);
		}
	});

	test('returns an empty array without a section for the command', () => {
		expect(to_forwarded_argv('tool', to_raw_rest_args(['node', 'gro', 'x']))).toEqual([]);
		expect(forward()).toEqual([]);
		expect(to_forwarded_argv('tsc', to_raw_rest_args('gro x -- eslint --fix'.split(' ')))).toEqual(
			[]
		);
	});

	test('reads the section of its command among several, the last when repeated', () => {
		const raw_rest_args = to_raw_rest_args(
			'gro x -- --implicit -- eslint --fix -- gro test --a -- tsc -b -- eslint --cache'.split(' ')
		);
		expect(to_forwarded_argv('eslint', raw_rest_args)).toEqual(['--cache']);
		expect(to_forwarded_argv('tsc', raw_rest_args)).toEqual(['-b']);
	});

	test('a command named only as a later arg is no section', () => {
		const raw_rest_args = to_raw_rest_args('gro x -- --tool eslint --fix'.split(' '));
		expect(to_forwarded_argv('eslint', raw_rest_args)).toEqual([]);
	});
});

describe('to_forwarded_flag_value', () => {
	test('reads the value of each form', () => {
		expect(to_forwarded_flag_value(['--output', 'a'], 'output', 'o')).toBe('a');
		expect(to_forwarded_flag_value(['--output=a'], 'output', 'o')).toBe('a');
		expect(to_forwarded_flag_value(['--output='], 'output', 'o')).toBe('');
		expect(to_forwarded_flag_value(['-o', 'a'], 'output', 'o')).toBe('a');
		expect(to_forwarded_flag_value(['-oa'], 'output', 'o')).toBe('a');
	});

	test('the last one wins', () => {
		expect(to_forwarded_flag_value(['-o', 'a', '--x', '--output=b'], 'output', 'o')).toBe('b');
		expect(to_forwarded_flag_value(['--output=b', '-oc'], 'output', 'o')).toBe('c');
	});

	test('null when absent or without a value', () => {
		expect(to_forwarded_flag_value(['-wo', 'd'], 'output', 'o')).toBe(null); // groups aren't read
		expect(to_forwarded_flag_value([], 'output', 'o')).toBe(null);
		expect(to_forwarded_flag_value(['--outputs', 'a', '-x', 'o'], 'output', 'o')).toBe(null);
		expect(to_forwarded_flag_value(['-oa'], 'output')).toBe(null);
		expect(to_forwarded_flag_value(['--output'], 'output', 'o')).toBe(null);
	});
});

describe('has_forwarded_flag', () => {
	test('matches the long forms', () => {
		for (const arg of [
			'--max-warnings',
			'--max-warnings=1',
			'--no-max-warnings',
			'--no-max-warnings=x'
		]) {
			expect(has_forwarded_flag(['a', arg, 'b'], 'max-warnings')).toBe(true);
		}
	});

	test('ignores other flags, values, and positionals', () => {
		for (const argv of [
			['--max-warnings-x'],
			['--max'],
			['max-warnings'],
			['-max-warnings'],
			['--grep', 'max-warnings'],
			['--x=--max-warnings']
		]) {
			expect(has_forwarded_flag(argv, 'max-warnings')).toBe(false);
		}
	});

	test('matches case-insensitively when asked', () => {
		expect(has_forwarded_flag(['--noemit'], 'noEmit')).toBe(false);
		expect(has_forwarded_flag(['--noemit'], 'noEmit', true)).toBe(true);
		expect(has_forwarded_flag(['--NOEMIT=false'], 'noEmit', true)).toBe(true);
	});

	test('a one-character name matches a short flag starting with it', () => {
		expect(has_forwarded_flag(['-o'], 'o')).toBe(true);
		expect(has_forwarded_flag(['-odist'], 'o')).toBe(true);
		expect(has_forwarded_flag(['--o'], 'o')).toBe(true);
		expect(has_forwarded_flag(['--no-o'], 'o')).toBe(true);
		expect(has_forwarded_flag(['-wo'], 'o')).toBe(false);
		expect(has_forwarded_flag(['-'], 'o')).toBe(false);
		expect(has_forwarded_flag(['-o'], 'output')).toBe(false);
	});
});

describe('to_implicit_forwarded_argv', () => {
	const implicit = (cli: string, task_name = 'test', command?: string): Array<string> =>
		to_implicit_forwarded_argv(task_name, command, ['node', 'gro', ...cli.split(' ')]);

	test('the first section verbatim for the task the CLI invoked', () => {
		expect(implicit('test -- --watch --coverage')).toEqual(['--watch', '--coverage']);
		expect(implicit('test -- foo.test.ts --max-workers 01')).toEqual([
			'foo.test.ts',
			'--max-workers',
			'01'
		]);
	});

	test('only the first section', () => {
		expect(implicit('test -- --watch -- eslint --fix')).toEqual(['--watch']);
		expect(implicit('test -- -- --watch')).toEqual([]);
	});

	test('none for a nested task', () => {
		expect(implicit('check -- --watch')).toEqual([]);
		expect(implicit('check -- gro test --coverage')).toEqual([]);
	});

	test('none for a first section of a gro task or the explicit command', () => {
		expect(implicit('test -- gro test --coverage')).toEqual([]);
		expect(implicit('test -- vitest --watch', 'test', 'vitest')).toEqual([]);
		expect(implicit('test -- --watch vitest', 'test', 'vitest')).toEqual(['--watch', 'vitest']);
	});

	test('none without a `--`', () => {
		expect(implicit('test --watch')).toEqual([]);
		expect(implicit('test --')).toEqual([]);
	});

	test('a project task is matched by its normalized path', () => {
		expect(implicit('task ./src/lib/a.task.ts -- --x', 'src/lib/a.task.ts')).toEqual(['--x']);
		expect(implicit('task src/lib/a.task.ts -- --x', 'src/lib/b.task.ts')).toEqual([]);
		expect(implicit('task -- --x', 'task')).toEqual([]);
	});
});

describe('to_task_args', () => {
	test('basic task with flags', () => {
		const result = to_task_args(['node', 'gro', 'test', '--watch']);
		expect(result).toEqual({
			task_name: 'test',
			task_path: null,
			args: { watch: true }
		});
	});

	test('task with positionals', () => {
		const result = to_task_args(['node', 'gro', 'test', 'foo.ts', 'bar.ts']);
		expect(result).toEqual({
			task_name: 'test',
			task_path: null,
			args: { _: ['foo.ts', 'bar.ts'] }
		});
	});

	test('stops parsing at --', () => {
		const result = to_task_args(['node', 'gro', 'test', '--watch', '--', 'vitest', '--coverage']);
		expect(result).toEqual({
			task_name: 'test',
			task_path: null,
			args: { watch: true }
		});
	});

	test('no task name', () => {
		const result = to_task_args(['node', 'gro', '--help']);
		expect(result).toEqual({
			task_name: '',
			task_path: null,
			args: { help: true }
		});
	});

	test('empty _ array removed to enable schema defaults', () => {
		const result = to_task_args(['node', 'gro', 'test']);
		expect(result).toEqual({
			task_name: 'test',
			task_path: null,
			args: {}
		});
		expect(result.args._).toBeUndefined();
	});

	test('task name from positionals', () => {
		const result = to_task_args(['node', 'gro', 'build']);
		expect(result).toEqual({
			task_name: 'build',
			task_path: null,
			args: {}
		});
	});

	test('mixed flags and positionals', () => {
		const result = to_task_args(['node', 'gro', 'test', 'foo.ts', 'bar.ts', '--watch']);
		expect(result).toEqual({
			task_name: 'test',
			task_path: null,
			args: { _: ['foo.ts', 'bar.ts'], watch: true }
		});
	});

	test('`gro task` consumes the path after it', () => {
		const result = to_task_args(['node', 'gro', 'task', 'src/lib/a.task.ts', 'b', '--c']);
		expect(result).toEqual({
			task_name: 'task',
			task_path: 'src/lib/a.task.ts',
			args: { _: ['b'], c: true }
		});
	});

	test('`gro task` with no path', () => {
		const result = to_task_args(['node', 'gro', 'task', '--c']);
		expect(result).toEqual({ task_name: 'task', task_path: null, args: { c: true } });
		expect(result.args._).toBeUndefined();
	});

	test('`task` after a builtin name is a positional, not the verb', () => {
		const result = to_task_args(['node', 'gro', 'test', 'task']);
		expect(result).toEqual({ task_name: 'test', task_path: null, args: { _: ['task'] } });
	});

	test('a flag replacing the positional args is a usage error', () => {
		for (const argv of [['run', '--no-_'], ['run', '--_', 'x.ts'], ['-_']]) {
			expect(() => to_task_args(['node', 'gro', ...argv])).toThrow(TaskError);
			expect(() => to_task_args(['node', 'gro', ...argv])).toThrow(
				"`_` holds the positional args and can't be set by a flag like `--_` or `--no-_`"
			);
		}
	});
});

describe('to_task_args for `gro run`', () => {
	test("everything after the path is the script's, verbatim", () => {
		const result = to_task_args([
			'node',
			'gro',
			'run',
			'x.ts',
			'--help',
			'--port',
			'0123',
			'--no-cache',
			'--',
			'--x=',
			'--_'
		]);
		expect(result).toEqual({
			task_name: 'run',
			task_path: null,
			args: { _: ['x.ts', '--help', '--port', '0123', '--no-cache', '--', '--x=', '--_'] }
		});
		expect(to_raw_rest_args(['node', 'gro', 'run', 'x.ts', '--', 'gro', 'test', '--a'])).toEqual(
			[]
		);
	});

	test('only `--help` goes before the path', () => {
		expect(to_task_args(['node', 'gro', 'run', '--help'])).toEqual({
			task_name: 'run',
			task_path: null,
			args: { help: true }
		});
		expect(to_task_args(['node', 'gro', 'run', '--help', 'x.ts', '--b'])).toEqual({
			task_name: 'run',
			task_path: null,
			args: { _: ['x.ts', '--b'], help: true }
		});
	});

	test('any other flag before the path is a usage error naming the fix', () => {
		for (const [argv, flags] of [
			[['--foo', 'x.ts', 'a'], '--foo'],
			[['--foo', 'bar', 'x.ts'], '--foo'],
			[['-f', '--port=1', 'x.ts'], '-f --port=1'],
			[['--no-x', '--help', 'x.ts'], '--no-x']
		] as const) {
			expect(() => to_task_args(['node', 'gro', 'run', ...argv])).toThrow(TaskError);
			expect(() => to_task_args(['node', 'gro', 'run', ...argv])).toThrow(
				`flags for the script go after it: \`gro run <path> ${flags}\``
			);
		}
	});

	test('a `--` before any path ends the CLI args as for other tasks', () => {
		expect(to_task_args(['node', 'gro', 'run', '--', 'x.ts'])).toEqual({
			task_name: 'run',
			task_path: null,
			args: {}
		});
		expect(to_raw_rest_args(['node', 'gro', 'run', '--', 'x.ts'])).toEqual(['--', 'x.ts']);
	});

	test('`run` not as the first arg is a positional', () => {
		expect(to_task_args(['node', 'gro', 'test', 'run', 'x.ts', '--a'])).toEqual({
			task_name: 'test',
			task_path: null,
			args: { _: ['run', 'x.ts'], a: true }
		});
	});
});

describe('to_raw_rest_args', () => {
	test('with --', () => {
		const result = to_raw_rest_args(['node', 'gro', 'test', '--', 'foo']);
		expect(result).toEqual(['--', 'foo']);
	});

	test('without --', () => {
		const result = to_raw_rest_args(['node', 'gro', 'test']);
		expect(result).toEqual([]);
	});

	test('multiple --', () => {
		const result = to_raw_rest_args(['node', 'gro', 'test', '--', 'foo', '--', 'bar']);
		expect(result).toEqual(['--', 'foo', '--', 'bar']);
	});

	test('only --', () => {
		const result = to_raw_rest_args(['node', 'gro', 'test', '--']);
		expect(result).toEqual(['--']);
	});

	test('args before -- ignored', () => {
		const result = to_raw_rest_args(['node', 'gro', 'test', '--watch', '--', '--coverage']);
		expect(result).toEqual(['--', '--coverage']);
	});
});

describe('to_forwarded_args', () => {
	test('returns args for a gro task command', () => {
		const raw_rest_args = to_raw_rest_args('gro check -- gro test --fix'.split(' '));
		expect(to_forwarded_args('gro test', raw_rest_args)).toEqual({ fix: true });
	});

	test('returns empty object for a tool or absent command', () => {
		const raw_rest_args = to_raw_rest_args('gro check -- eslint --fix'.split(' '));
		expect(to_forwarded_args('eslint', raw_rest_args)).toEqual({});
		expect(to_forwarded_args('gro lint', raw_rest_args)).toEqual({});
	});
});

describe('merge_forwarded_args', () => {
	const ColorArgs = s.object({
		_: s.with_default_fn(s.array(s.string), () => []),
		color: meta(s.with_default(s.boolean, true), { aliases: ['c'] }),
		'no-color': meta(s.with_default(s.boolean, false), { aliases: ['C'] })
	});

	test('a forwarded flag wins a no-x pair against a later direct alias', () => {
		// `gro x --color -C -- gro x --color`
		const argv = 'gro x --color -C -- gro x --color'.split(' ');
		const { args } = to_task_args(['node', ...argv]);
		const forwarded = to_forwarded_args('gro x', to_raw_rest_args(['node', ...argv]));
		const merged = merge_forwarded_args(args, forwarded);
		expect(Object.keys(merged)).toEqual(['C', 'color']);
		const parsed = args_parse(merged, ColorArgs);
		expect(parsed.ok && parsed.value).toEqual({ _: [], color: true, 'no-color': false });
	});

	test('a forwarded no-x wins against a direct x', () => {
		const merged = merge_forwarded_args({ 'no-color': true, color: true }, { 'no-color': true });
		const parsed = args_parse(merged, ColorArgs);
		expect(parsed.ok && parsed.value).toEqual({ _: [], color: false, 'no-color': true });
	});

	test('a forwarded non-pair key overrides the direct value and keeps the others', () => {
		const args = { a: 1, b: 'x', c: true };
		const merged = merge_forwarded_args(args, { b: 'y', d: 4 });
		expect(merged).toEqual({ a: 1, b: 'y', c: true, d: 4 });
		expect(Object.keys(merged)).toEqual(['a', 'c', 'b', 'd']);
		expect(args).toEqual({ a: 1, b: 'x', c: true }); // not mutated
	});

	test('an own `__proto__` arg stays an own key', () => {
		const direct = argv_parse(['--__proto__', 'a']);
		const forwarded = argv_parse(['--__proto__', 'b']);
		const merged = merge_forwarded_args(direct, forwarded);
		expect(Object.hasOwn(merged, '__proto__')).toBe(true);
		expect(Object.getPrototypeOf(merged)).toBe(Object.prototype);
		expect(Object.getOwnPropertyDescriptor(merged, '__proto__')?.value).toBe('b');
		const kept = merge_forwarded_args(direct, {});
		expect(Object.getOwnPropertyDescriptor(kept, '__proto__')?.value).toBe('a');
	});

	test('undefined direct args and empty forwarded args', () => {
		expect(merge_forwarded_args(undefined, { a: 1 })).toEqual({ a: 1 });
		const args = { a: 1 };
		const merged = merge_forwarded_args(args, {});
		expect(merged).toEqual({ a: 1 });
		expect(merged).not.toBe(args);
	});
});

describe('integration: round-trip parsing', () => {
	test('serialize and parse produces same args', () => {
		// Note: false boolean values cannot round-trip (--flag is always true in argv_parse)
		const original = { _: ['foo', 'bar'], watch: true, count: 3 };
		const serialized = args_serialize(original);
		const reparsed = argv_parse(serialized);
		expect(reparsed).toEqual(original);
	});

	test('complex args round-trip', () => {
		const original = { _: ['a', 'b'], flag: ['x', 'y', 'z'], w: true, count: 42 };
		const serialized = args_serialize(original);
		const reparsed = argv_parse(serialized);
		expect(reparsed).toEqual(original);
	});

	test('empty args round-trip', () => {
		const original = {};
		const serialized = args_serialize(original);
		const reparsed = argv_parse(serialized);
		// argv_parse always adds _ array
		expect(reparsed).toEqual({ _: [] });
	});

	test('only positionals round-trip', () => {
		const original = { _: ['src', 'lib', 'test'] };
		const serialized = args_serialize(original);
		const reparsed = argv_parse(serialized);
		expect(reparsed).toEqual(original);
	});

	test('no- prefix round-trip', () => {
		// --no-watch is parsed as {watch: false} by argv_parse
		// This means serialization of {no-watch: true} cannot round-trip perfectly
		// since --no-watch becomes {watch: false}, not {no-watch: true}
		const original = { _: [], 'no-watch': true };
		const serialized = args_serialize(original);
		expect(serialized).toEqual(['--no-watch']);
		const reparsed = argv_parse(serialized);
		// The --no-watch flag becomes {watch: false}, not {no-watch: true}
		expect(reparsed.watch).toBe(false);
		expect(reparsed['no-watch']).toBeUndefined();
	});

	test('string values with spaces require quoting at shell level', () => {
		// args_serialize produces separate array elements
		// shell would need to quote them, but argv_parse sees them as separate
		const original = { _: [], message: 'hello world' };
		const serialized = args_serialize(original);
		expect(serialized).toEqual(['--message', 'hello world']);
		// When passed as array to argv_parse (simulating proper shell quoting), it works
		const reparsed = argv_parse(serialized);
		expect(reparsed.message).toBe('hello world');
	});
});
