import type { Gen } from '#lib/gen.ts';
import { to_grant_flags } from '#lib/task_args.ts';
import { is_task_error } from '#lib/task_error.ts';

/*

Writes the fixture the binary's twin of `to_grant_flags` is held to:
each case's CLI args, the args after the CLI's own path, and what `to_grant_flags` makes of them,
the split or the refusal. The cases are here and the results are `to_grant_flags`'s,
so the fixture is never edited by hand, and `gro gen --check` fails when the function moves
until it's regenerated, the twin's test then failing where it hasn't followed.

A refusal is written as its kind, the flag it names, and its message. The kind and the flag
are each case's to declare, since a `TaskError` carries only its message, and this checks the
declaration against the message, so the twin compares kinds and flags, not the TS wording.

*/

/**
 * Why `to_grant_flags` refuses a case's args.
 */
type RefusalKind =
	| 'before_command'
	| 'takes_no_value'
	| 'takes_a_list'
	| 'no_grant_flag'
	| 'run_env_without_run'
	| 'run_write_without_run';

/**
 * A refusal as a case declares it: its kind, and the flag it names, as written,
 * `null` for `run_env_without_run` and `run_write_without_run`, which name none;
 * for `before_command`, the command its hint names, `null` for `<command>`.
 */
type Refusal =
	| { kind: 'before_command'; flag: string; command: string | null }
	| {
			kind: Exclude<
				RefusalKind,
				'before_command' | 'run_env_without_run' | 'run_write_without_run'
			>;
			flag: string;
	  }
	| { kind: 'run_env_without_run' | 'run_write_without_run' };

interface GrantFlagsCase {
	name: string;
	argv: Array<string>;
	refusal?: Refusal;
}

const before_command = (flag: string, command: string | null): Refusal => ({
	kind: 'before_command',
	flag,
	command
});
const takes_no_value = (flag: string): Refusal => ({ kind: 'takes_no_value', flag });
const takes_a_list = (flag: string): Refusal => ({ kind: 'takes_a_list', flag });
const no_grant_flag = (flag: string): Refusal => ({ kind: 'no_grant_flag', flag });
const RUN_ENV_WITHOUT_RUN: Refusal = { kind: 'run_env_without_run' };
const RUN_WRITE_WITHOUT_RUN: Refusal = { kind: 'run_write_without_run' };

const CASES: Array<GrantFlagsCase> = [
	// no grant flags
	{ name: 'empty', argv: [] },
	{ name: 'command_alone', argv: ['check'] },
	{ name: 'command_with_task_args', argv: ['check', '--watch', '--build'] },
	{ name: 'version_flag', argv: ['--version'] },
	// the run right after the command word
	{ name: 'all', argv: ['check', '-A'] },
	{ name: 'all_with_value', argv: ['check', '-A=x'] },
	{ name: 'temp', argv: ['check', '--allow-temp'] },
	{
		name: 'temp_with_value',
		argv: ['check', '--allow-temp=x'],
		refusal: takes_no_value('--allow-temp=x')
	},
	{
		name: 'temp_with_empty_value',
		argv: ['check', '--allow-temp='],
		refusal: takes_no_value('--allow-temp=')
	},
	{ name: 'read', argv: ['check', '--allow-read=.'] },
	{ name: 'read_list', argv: ['check', '--allow-read=a,b/c'] },
	{ name: 'read_value_with_equals', argv: ['check', '--allow-read=a=b'] },
	{ name: 'read_repeated', argv: ['check', '--allow-read=a', '--allow-read=b'] },
	// only the spelling is checked: a list's names are the grant's to refuse
	{ name: 'read_list_of_empty_names', argv: ['check', '--allow-read=,'] },
	{
		name: 'read_without_value',
		argv: ['check', '--allow-read'],
		refusal: takes_a_list('--allow-read')
	},
	{
		name: 'read_with_empty_value',
		argv: ['check', '--allow-read='],
		refusal: takes_a_list('--allow-read=')
	},
	{ name: 'write', argv: ['check', '--allow-write=.'] },
	{
		name: 'write_without_value',
		argv: ['check', '--allow-write'],
		refusal: takes_a_list('--allow-write')
	},
	{ name: 'env', argv: ['check', '--allow-env=CI,HOME'] },
	{
		name: 'env_without_value',
		argv: ['check', '--allow-env'],
		refusal: takes_a_list('--allow-env')
	},
	{ name: 'run', argv: ['check', '--allow-run=git'] },
	{
		name: 'run_without_value',
		argv: ['check', '--allow-run'],
		refusal: takes_a_list('--allow-run')
	},
	{ name: 'run_in', argv: ['check', '--allow-run-in=node_modules/.bin'] },
	{
		name: 'run_in_without_value',
		argv: ['check', '--allow-run-in'],
		refusal: takes_a_list('--allow-run-in')
	},
	{ name: 'run_env', argv: ['check', '--allow-run-env=NODE_OPTIONS'] },
	{
		name: 'run_env_without_value',
		argv: ['check', '--allow-run-env'],
		refusal: takes_a_list('--allow-run-env')
	},
	{ name: 'run_write', argv: ['test', '--allow-run-write=vitest:src', '--', '-u'] },
	{
		name: 'run_write_repeated',
		argv: ['check', '--allow-run-write=vitest:src', '--allow-run-write=eslint:src,test']
	},
	// the program and directories aren't read here, the binary's to check
	{ name: 'run_write_unread', argv: ['test', '--allow-run-write=src'] },
	{
		name: 'run_write_without_value',
		argv: ['test', '--allow-run-write'],
		refusal: takes_a_list('--allow-run-write')
	},
	{
		name: 'run_write_empty',
		argv: ['test', '--allow-run-write='],
		refusal: takes_a_list('--allow-run-write=')
	},
	// a flag's name is matched whole, so a longer one is no grant flag
	{
		name: 'run_write_misspelled',
		argv: ['test', '--allow-run-writes=vitest:src'],
		refusal: no_grant_flag('--allow-run-writes=vitest:src')
	},
	{
		name: 'every_flag',
		argv: [
			'check',
			'-A',
			'--allow-temp',
			'--allow-read=.',
			'--allow-write=.',
			'--allow-env=CI',
			'--allow-run=git',
			'--allow-run-in=bin',
			'--allow-run-env=X',
			'--allow-run-write=git:out',
			'--watch'
		]
	},
	{ name: 'run_then_task_args', argv: ['check', '--allow-read=.', '--watch', 'x'] },
	{ name: 'run_then_forwarded', argv: ['check', '-A', '--', 'gro', 'test', '--allow-read=.'] },
	// lookalikes: spelled `--allow-*` in the run, refused, or not spelled so, ending the run
	{
		name: 'lookalike_net',
		argv: ['check', '--allow-net=x'],
		refusal: no_grant_flag('--allow-net=x')
	},
	{
		name: 'lookalike_bare_prefix',
		argv: ['check', '--allow-'],
		refusal: no_grant_flag('--allow-')
	},
	{
		name: 'lookalike_case',
		argv: ['check', '--allow-READ=.'],
		refusal: no_grant_flag('--allow-READ=.')
	},
	{
		name: 'lookalike_all_long',
		argv: ['check', '--allow-all'],
		refusal: no_grant_flag('--allow-all')
	},
	{ name: 'lookalike_no_dash', argv: ['check', '--allowread=.', '--allow-read=.'] },
	{ name: 'lookalike_short', argv: ['check', '-a', '--allow-read=.'] },
	{ name: 'lookalike_short_group', argv: ['check', '-AA'] },
	{ name: 'lookalike_single_dash', argv: ['check', '-allow-read=.'] },
	{
		name: 'lookalike_after_valid',
		argv: ['check', '--allow-read=.', '--allow-net=x'],
		refusal: no_grant_flag('--allow-net=x')
	},
	// `--help` in the run steps over it, kept for the command
	{ name: 'help_alone', argv: ['check', '--help'] },
	{ name: 'help_before', argv: ['check', '--help', '--allow-read=.', '--watch'] },
	{ name: 'help_between', argv: ['check', '--allow-read=.', '--help', '--allow-write=.'] },
	{ name: 'help_after', argv: ['check', '-A', '--help', '--watch'] },
	{ name: 'help_twice', argv: ['check', '--help', '-A', '--help'] },
	{ name: 'help_short_ends_run', argv: ['check', '-h', '-A'] },
	// only `--help` itself steps over the run: a value or a `no-` ends it, as any other flag does
	{ name: 'help_with_value_ends_run', argv: ['check', '--help=x', '-A'] },
	{ name: 'help_negated_ends_run', argv: ['check', '--no-help', '-A'] },
	// a bare `-` is no grant flag, ending the run, and passed over in the scan before the command
	{ name: 'dash_ends_run', argv: ['check', '-', '-A'] },
	{
		name: 'dash_before_command',
		argv: ['-', '-A', 'check'],
		refusal: before_command('-A', 'check')
	},
	// flags past the run, or in a `--` section, are the command's or forwarded as written
	{ name: 'after_task_arg', argv: ['check', '--watch', '--allow-read=.'] },
	{ name: 'after_positional', argv: ['check', 'x', '-A'] },
	{ name: 'in_forwarded_section', argv: ['check', '--', '--allow-read=.'] },
	{ name: 'in_gro_section', argv: ['check', '--', 'gro', 'check', '-A'] },
	{ name: 'forwarded_alone', argv: ['--', '--allow-read=.'] },
	{ name: 'forwarded_lookalike', argv: ['check', '--', '--allow-net=x'] },
	// before the command word
	{
		name: 'before_command',
		argv: ['--allow-read=.', 'check'],
		refusal: before_command('--allow-read=.', 'check')
	},
	{ name: 'all_before_command', argv: ['-A', 'check'], refusal: before_command('-A', 'check') },
	{ name: 'all_alone', argv: ['-A'], refusal: before_command('-A', null) },
	{
		name: 'before_command_after_flag',
		argv: ['--watch', '--allow-read=.', 'check'],
		refusal: before_command('--allow-read=.', 'check')
	},
	{ name: 'before_forwarded', argv: ['-A', '--', 'check'], refusal: before_command('-A', null) },
	{
		name: 'lookalike_before_command',
		argv: ['--allow-net=x', 'check'],
		refusal: before_command('--allow-net=x', 'check')
	},
	{
		name: 'before_path',
		argv: ['-A', 'src/x.task.ts'],
		refusal: before_command('-A', 'src/x.task.ts')
	},
	// a path in the command slot is no command word, so there's no run
	{ name: 'path_command', argv: ['src/x.task.ts', '--allow-read=.'] },
	{ name: 'path_command_bare_task_file', argv: ['x.task.ts', '-A'] },
	{ name: 'path_command_js_task_file', argv: ['x.task.js', '-A'] },
	{ name: 'path_command_slash', argv: ['a/b', '-A'] },
	{ name: 'path_command_dot', argv: ['./check', '-A'] },
	{ name: 'path_command_lookalike', argv: ['x.task.ts', '--allow-net=x'] },
	{ name: 'not_a_path_ts', argv: ['x.ts', '-A'] },
	{ name: 'empty_command', argv: ['', '-A'] },
	// `gro task` and `gro run`
	{ name: 'task_path_after_run', argv: ['task', '--allow-read=.', 'src/x.task.ts', '--a'] },
	{ name: 'task_listing', argv: ['task', '-A'] },
	{ name: 'run_script', argv: ['run', '--allow-read=.', 'x.ts', '--allow-write=.'] },
	{ name: 'run_help', argv: ['run', '--help', '--allow-read=.', 'x.ts'] },
	{
		name: 'run_run_env_alone',
		argv: ['run', '--allow-run-env=X', 'x.ts'],
		refusal: RUN_ENV_WITHOUT_RUN
	},
	{ name: 'run_run_env_with_run', argv: ['run', '--allow-run-env=X', '--allow-run=git', 'x.ts'] },
	{
		name: 'run_run_env_with_run_in',
		argv: ['run', '--allow-run-in=bin', '--allow-run-env=X', 'x.ts']
	},
	{
		name: 'run_run_env_with_all',
		argv: ['run', '-A', '--allow-run-env=X', 'x.ts'],
		refusal: RUN_ENV_WITHOUT_RUN
	},
	{ name: 'run_run_env_after_run', argv: ['run', 'x.ts', '--allow-run-env=X'] },
	{
		name: 'run_run_write_alone',
		argv: ['run', '--allow-run-write=x:out', 'x.ts'],
		refusal: RUN_WRITE_WITHOUT_RUN
	},
	{
		name: 'run_run_write_with_run_in',
		argv: ['run', '--allow-run-in=bin', '--allow-run-write=x:out', 'x.ts'],
		refusal: RUN_WRITE_WITHOUT_RUN
	},
	{
		name: 'run_run_write_with_all',
		argv: ['run', '-A', '--allow-run-write=x:out', 'x.ts'],
		refusal: RUN_WRITE_WITHOUT_RUN
	},
	{
		name: 'run_run_write_with_run',
		argv: ['run', '--allow-run=x', '--allow-run-write=x:out', 'x.ts']
	},
	{ name: 'run_run_write_after_run', argv: ['run', 'x.ts', '--allow-run-write=x:out'] },
	{ name: 'check_run_write_alone', argv: ['check', '--allow-run-write=vitest:src'] },
	{ name: 'check_run_env_alone', argv: ['check', '--allow-run-env=X'] }
];

/**
 * The words a message of each kind holds, checking a case's declared kind against `to_grant_flags`.
 */
const REFUSAL_WORDS: Record<RefusalKind, string> = {
	before_command: 'comes before the command',
	takes_no_value: 'takes no value',
	takes_a_list: 'takes a list after `=`',
	no_grant_flag: 'is no grant flag',
	run_env_without_run: 'passes variables to the programs',
	run_write_without_run: 'lets a program `--allow-run` grants write'
};

/**
 * Throws unless `message` is the refusal `refusal` declares: its kind's words,
 * the flag it names in backticks where the message names it, first or last, since a
 * message that lists names its flag's name too, and for `before_command` the hint's command.
 */
const check_refusal = (name: string, refusal: Refusal, message: string): void => {
	const holds = (() => {
		if (!message.includes(REFUSAL_WORDS[refusal.kind])) return false;
		switch (refusal.kind) {
			case 'run_env_without_run':
			case 'run_write_without_run':
				return true;
			case 'takes_no_value':
			case 'takes_a_list':
				return message.endsWith(`: \`${refusal.flag}\``);
			case 'no_grant_flag':
				return message.startsWith(`\`${refusal.flag}\` `);
			case 'before_command':
				return (
					message.startsWith(`\`${refusal.flag}\` `) &&
					message.endsWith(`\`gro ${refusal.command ?? '<command>'} ${refusal.flag}\``)
				);
		}
	})();
	if (!holds)
		throw Error(`grant flag case ${name} declares ${JSON.stringify(refusal)}: ${message}`);
};

/**
 * What `to_grant_flags` makes of a case's args: the split, or the refusal it declares,
 * with the message of the `TaskError` thrown.
 */
const to_result = ({
	name,
	argv,
	refusal
}: GrantFlagsCase):
	{ cli_args: Array<string>; grant_flags: Array<string> } | { refusal: Refusal; error: string } => {
	let split;
	try {
		split = to_grant_flags(argv);
	} catch (error) {
		if (!is_task_error(error)) throw error;
		if (!refusal) throw Error(`grant flag case ${name} declares no refusal: ${error.message}`);
		check_refusal(name, refusal, error.message);
		return { refusal, error: error.message };
	}
	if (refusal) throw Error(`grant flag case ${name} declares a refusal it doesn't get`);
	return { cli_args: split.cli_args, grant_flags: split.grant_flags };
};

export const gen: Gen = ({ origin_path }) => {
	const names: Set<string> = new Set();
	for (const { name } of CASES) {
		if (names.has(name)) throw Error(`two grant flag cases are named ${name}`);
		names.add(name);
	}
	const fixture = {
		description:
			"Each case's CLI args, the args after the CLI's own path, and what `to_grant_flags` " +
			'(`src/lib/task_args.ts`) makes of them: the grant flags split off and the CLI args left, ' +
			'or its refusal, the kind and flag the case declares, checked against the message it refuses with. ' +
			`Generated by ${origin_path}, never edited by hand.`,
		cases: CASES.map((c) => ({ name: c.name, argv: c.argv, ...to_result(c) }))
	};
	return {
		filename: '../../crates/gro_cli/tests/fixtures/verb/grant_flags.json',
		content: JSON.stringify(fixture, null, '\t') + '\n'
	};
};
