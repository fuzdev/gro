import { args_parse, args_serialize, argv_parse } from '@fuzdev/fuz_util/args.ts';
import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';
import { describe, test, expect } from 'vitest';
import { resolve } from 'node:path';

import {
	merge_forwarded_args,
	to_forwarded_args,
	to_forwarded_args_by_command,
	to_raw_rest_args,
	to_implicit_forwarded_args,
	to_task_args
} from '#lib/args.ts';
import { TaskError } from '#lib/task.ts';

describe('to_forwarded_args_by_command', () => {
	test('basic behavior', () => {
		const raw_rest_args = to_raw_rest_args(
			(
				'gro taskname a b c --d -e 1 --     --    ' +
				'eslint  a  --b    c --  ' +
				'gro  a --a  --   ' +
				'tsc  -b  --    ' +
				'gro b -t2 t2a --t2 t2b --t222 2 --     --      --    ' +
				'groc --m --n nn -- ' +
				'gro d -b a --c 4 -- ' +
				'gro d -b a --c 5 -- '
			).split(' ')
		);
		expect(to_forwarded_args_by_command(raw_rest_args)).toEqual({
			eslint: { _: ['a'], b: 'c' },
			'gro a': { a: true },
			tsc: { b: true },
			'gro b': { '2': 't2a', t: true, t2: 't2b', t222: 2 },
			groc: { m: true, n: 'nn' },
			'gro d': { b: 'a', c: 5 }
		});
		expect(to_forwarded_args('gro b', raw_rest_args)).toEqual({
			'2': 't2a',
			t: true,
			t2: 't2b',
			t222: 2
		});
	});

	test('skips sections without command name (handled by to_implicit_forwarded_args)', () => {
		// Sections starting with flags (no command) are skipped, not thrown
		// This allows `gro run script.ts -- --help` to pass `--help` to the script
		const raw_rest_args = to_raw_rest_args('gro test -- --flag'.split(' '));
		expect(to_forwarded_args_by_command(raw_rest_args)).toEqual({});
	});

	test('skips command-less sections but parses command sections', () => {
		// Mixed: one section has command, one doesn't
		const raw_rest_args = to_raw_rest_args('gro test -- --flag -- eslint --fix'.split(' '));
		expect(to_forwarded_args_by_command(raw_rest_args)).toEqual({ eslint: { fix: true } });
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

	test('a forwarded flag replacing the positional args is a usage error', () => {
		const raw_rest_args = to_raw_rest_args('gro test -- vitest --no-_'.split(' '));
		expect(() => to_forwarded_args_by_command(raw_rest_args)).toThrow(TaskError);
		expect(() => to_implicit_forwarded_args(undefined, raw_rest_args)).toThrow(TaskError);
	});

	test('throws when gro with only flags', () => {
		const raw_rest_args = to_raw_rest_args('gro test -- gro --flag'.split(' '));
		expect(() => to_forwarded_args_by_command(raw_rest_args)).toThrow(
			'Malformed args following a `--`. Expected gro taskname'
		);
	});
});

describe('to_implicit_forwarded_args', () => {
	test('implicit flags only', () => {
		const raw_rest_args = to_raw_rest_args('gro test -- --watch --coverage'.split(' '));
		expect(to_implicit_forwarded_args(undefined, raw_rest_args)).toEqual({
			watch: true,
			coverage: true
		});
	});

	test('positional args only', () => {
		const raw_rest_args = to_raw_rest_args('gro test -- foo.test.ts bar.test.ts'.split(' '));
		expect(to_implicit_forwarded_args(undefined, raw_rest_args)).toEqual({
			_: ['foo.test.ts', 'bar.test.ts']
		});
	});

	test('command stripping for backward compat', () => {
		const raw_rest_args = to_raw_rest_args('gro test -- vitest --watch --coverage'.split(' '));
		expect(to_implicit_forwarded_args('vitest', raw_rest_args)).toEqual({
			watch: true,
			coverage: true
		});
	});

	test('command stripping does nothing if command not present', () => {
		const raw_rest_args = to_raw_rest_args('gro test -- --watch --coverage'.split(' '));
		expect(to_implicit_forwarded_args('vitest', raw_rest_args)).toEqual({
			watch: true,
			coverage: true
		});
	});

	test('mixed flags and positionals', () => {
		const raw_rest_args = to_raw_rest_args('gro test -- foo.test.ts --watch --coverage'.split(' '));
		expect(to_implicit_forwarded_args(undefined, raw_rest_args)).toEqual({
			_: ['foo.test.ts'],
			watch: true,
			coverage: true
		});
	});

	test('no forwarded args', () => {
		const raw_rest_args = to_raw_rest_args('gro test'.split(' '));
		expect(to_implicit_forwarded_args(undefined, raw_rest_args)).toEqual({});
	});

	test('empty args after --', () => {
		const raw_rest_args = to_raw_rest_args('gro test --'.split(' '));
		expect(to_implicit_forwarded_args(undefined, raw_rest_args)).toEqual({});
	});

	test('parses all args after first -- (argv_parse handles -- separator)', () => {
		const raw_rest_args = to_raw_rest_args('gro test -- --watch -- eslint --fix'.split(' '));
		// argv_parse treats everything after first -- as input, and filters internal -- separators
		expect(to_implicit_forwarded_args(undefined, raw_rest_args)).toEqual({
			_: ['eslint', '--fix'],
			watch: true
		});
	});

	test('complex real-world example', () => {
		const raw_rest_args = to_raw_rest_args(
			'gro test -- vitest src/*.test.ts --watch --coverage'.split(' ')
		);
		expect(to_implicit_forwarded_args('vitest', raw_rest_args)).toEqual({
			_: ['src/*.test.ts'],
			watch: true,
			coverage: true
		});
	});

	test('command stripping with multiple -- sections', () => {
		const raw_rest_args = to_raw_rest_args('gro test -- vitest --watch -- other --flag'.split(' '));
		// Strips 'vitest', argv_parse handles the rest
		expect(to_implicit_forwarded_args('vitest', raw_rest_args)).toEqual({
			_: ['other', '--flag'],
			watch: true
		});
	});

	test('command stripping does not affect later positionals', () => {
		const raw_rest_args = to_raw_rest_args(
			'gro test -- vitest --watch -- vitest --another'.split(' ')
		);
		// Only strips first 'vitest', second one becomes positional
		expect(to_implicit_forwarded_args('vitest', raw_rest_args)).toEqual({
			_: ['vitest', '--another'],
			watch: true
		});
	});

	test('wrong command to strip with multiple sections', () => {
		const raw_rest_args = to_raw_rest_args('gro test -- eslint --fix -- vitest --watch'.split(' '));
		// command_to_strip='vitest' but first arg is 'eslint', so nothing stripped
		// argv_parse treats --fix as a flag, not a positional
		expect(to_implicit_forwarded_args('vitest', raw_rest_args)).toEqual({
			_: ['eslint', 'vitest', '--watch'],
			fix: true
		});
	});

	test('command name as flag value should not be stripped', () => {
		const raw_rest_args = to_raw_rest_args('gro test -- --tool vitest --watch'.split(' '));
		// 'vitest' is value for --tool flag, not first positional
		expect(to_implicit_forwarded_args('vitest', raw_rest_args)).toEqual({
			tool: 'vitest',
			watch: true
		});
	});

	test('numeric positionals', () => {
		const raw_rest_args = to_raw_rest_args('gro test -- 123 456 --count 789'.split(' '));
		// argv_parse keeps positionals as strings from argv, but coerces flag values as numbers
		expect(to_implicit_forwarded_args(undefined, raw_rest_args)).toEqual({
			_: ['123', '456'],
			count: 789
		});
	});

	test('flags with = syntax', () => {
		const raw_rest_args = to_raw_rest_args('gro test -- --foo=bar --baz=qux'.split(' '));
		expect(to_implicit_forwarded_args(undefined, raw_rest_args)).toEqual({
			foo: 'bar',
			baz: 'qux'
		});
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
		for (const argv of [['run', '--no-_'], ['run', 'x.ts', '--_'], ['-_']]) {
			expect(() => to_task_args(['node', 'gro', ...argv])).toThrow(TaskError);
			expect(() => to_task_args(['node', 'gro', ...argv])).toThrow(
				"`_` holds the positional args and can't be set by a flag like `--_` or `--no-_`"
			);
		}
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
	test('returns args for existing command', () => {
		const raw_rest_args = to_raw_rest_args('gro test -- eslint --fix'.split(' '));
		expect(to_forwarded_args('eslint', raw_rest_args)).toEqual({ fix: true });
	});

	test('returns empty object for non-existent command', () => {
		const raw_rest_args = to_raw_rest_args('gro test -- eslint --fix'.split(' '));
		expect(to_forwarded_args('tsc', raw_rest_args)).toEqual({});
	});

	test('uses provided cache', () => {
		const raw_rest_args = to_raw_rest_args('gro test -- eslint --fix'.split(' '));
		const cache = to_forwarded_args_by_command(raw_rest_args);
		// Same cache should be reused
		expect(to_forwarded_args('eslint', raw_rest_args, cache)).toEqual({ fix: true });
		expect(to_forwarded_args('tsc', raw_rest_args, cache)).toEqual({});
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
