import type { Gen } from '#lib/gen.ts';
import { to_grant_flags } from '#lib/task_args.ts';
import { is_task_error } from '#lib/task_error.ts';

/*

Writes the fixture the binary's twin of `to_grant_flags` is held to:
each case's CLI args, the args after the CLI's own path, and what `to_grant_flags` makes of them,
the split or the refusal's message. The cases are here and the results are `to_grant_flags`'s,
so the fixture is never edited by hand, and `gro gen --check` fails when the function moves
until it's regenerated, the twin's test then failing where it hasn't followed.

*/

interface GrantFlagsCase {
	name: string;
	argv: Array<string>;
}

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
	{ name: 'temp_with_value', argv: ['check', '--allow-temp=x'] },
	{ name: 'temp_with_empty_value', argv: ['check', '--allow-temp='] },
	{ name: 'read', argv: ['check', '--allow-read=.'] },
	{ name: 'read_list', argv: ['check', '--allow-read=a,b/c'] },
	{ name: 'read_value_with_equals', argv: ['check', '--allow-read=a=b'] },
	{ name: 'read_repeated', argv: ['check', '--allow-read=a', '--allow-read=b'] },
	{ name: 'read_without_value', argv: ['check', '--allow-read'] },
	{ name: 'read_with_empty_value', argv: ['check', '--allow-read='] },
	{ name: 'write', argv: ['check', '--allow-write=.'] },
	{ name: 'write_without_value', argv: ['check', '--allow-write'] },
	{ name: 'env', argv: ['check', '--allow-env=CI,HOME'] },
	{ name: 'env_without_value', argv: ['check', '--allow-env'] },
	{ name: 'run', argv: ['check', '--allow-run=git'] },
	{ name: 'run_without_value', argv: ['check', '--allow-run'] },
	{ name: 'run_in', argv: ['check', '--allow-run-in=node_modules/.bin'] },
	{ name: 'run_in_without_value', argv: ['check', '--allow-run-in'] },
	{ name: 'run_env', argv: ['check', '--allow-run-env=NODE_OPTIONS'] },
	{ name: 'run_env_without_value', argv: ['check', '--allow-run-env'] },
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
			'--watch'
		]
	},
	{ name: 'run_then_task_args', argv: ['check', '--allow-read=.', '--watch', 'x'] },
	{ name: 'run_then_forwarded', argv: ['check', '-A', '--', 'gro', 'test', '--allow-read=.'] },
	// lookalikes: spelled `--allow-*` in the run, refused, or not spelled so, ending the run
	{ name: 'lookalike_net', argv: ['check', '--allow-net=x'] },
	{ name: 'lookalike_bare_prefix', argv: ['check', '--allow-'] },
	{ name: 'lookalike_case', argv: ['check', '--allow-READ=.'] },
	{ name: 'lookalike_all_long', argv: ['check', '--allow-all'] },
	{ name: 'lookalike_no_dash', argv: ['check', '--allowread=.', '--allow-read=.'] },
	{ name: 'lookalike_short', argv: ['check', '-a', '--allow-read=.'] },
	{ name: 'lookalike_short_group', argv: ['check', '-AA'] },
	{ name: 'lookalike_single_dash', argv: ['check', '-allow-read=.'] },
	{ name: 'lookalike_after_valid', argv: ['check', '--allow-read=.', '--allow-net=x'] },
	// `--help` in the run steps over it, kept for the command
	{ name: 'help_alone', argv: ['check', '--help'] },
	{ name: 'help_before', argv: ['check', '--help', '--allow-read=.', '--watch'] },
	{ name: 'help_between', argv: ['check', '--allow-read=.', '--help', '--allow-write=.'] },
	{ name: 'help_after', argv: ['check', '-A', '--help', '--watch'] },
	{ name: 'help_twice', argv: ['check', '--help', '-A', '--help'] },
	{ name: 'help_short_ends_run', argv: ['check', '-h', '-A'] },
	// flags past the run, or in a `--` section, are the command's or forwarded as written
	{ name: 'after_task_arg', argv: ['check', '--watch', '--allow-read=.'] },
	{ name: 'after_positional', argv: ['check', 'x', '-A'] },
	{ name: 'in_forwarded_section', argv: ['check', '--', '--allow-read=.'] },
	{ name: 'in_gro_section', argv: ['check', '--', 'gro', 'check', '-A'] },
	{ name: 'forwarded_alone', argv: ['--', '--allow-read=.'] },
	{ name: 'forwarded_lookalike', argv: ['check', '--', '--allow-net=x'] },
	// before the command word
	{ name: 'before_command', argv: ['--allow-read=.', 'check'] },
	{ name: 'all_before_command', argv: ['-A', 'check'] },
	{ name: 'all_alone', argv: ['-A'] },
	{ name: 'before_command_after_flag', argv: ['--watch', '--allow-read=.', 'check'] },
	{ name: 'before_forwarded', argv: ['-A', '--', 'check'] },
	{ name: 'lookalike_before_command', argv: ['--allow-net=x', 'check'] },
	{ name: 'before_path', argv: ['-A', 'src/x.task.ts'] },
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
	{ name: 'run_run_env_alone', argv: ['run', '--allow-run-env=X', 'x.ts'] },
	{ name: 'run_run_env_with_run', argv: ['run', '--allow-run-env=X', '--allow-run=git', 'x.ts'] },
	{
		name: 'run_run_env_with_run_in',
		argv: ['run', '--allow-run-in=bin', '--allow-run-env=X', 'x.ts']
	},
	{ name: 'run_run_env_with_all', argv: ['run', '-A', '--allow-run-env=X', 'x.ts'] },
	{ name: 'run_run_env_after_run', argv: ['run', 'x.ts', '--allow-run-env=X'] },
	{ name: 'check_run_env_alone', argv: ['check', '--allow-run-env=X'] }
];

/**
 * What `to_grant_flags` makes of `argv`: the split, or the message of the `TaskError` it throws.
 */
const to_result = (
	argv: Array<string>
): { cli_args: Array<string>; grant_flags: Array<string> } | { error: string } => {
	try {
		const { cli_args, grant_flags } = to_grant_flags(argv);
		return { cli_args, grant_flags };
	} catch (error) {
		if (!is_task_error(error)) throw error;
		return { error: error.message };
	}
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
			`or the message it refuses them with. Generated by ${origin_path}, never edited by hand.`,
		cases: CASES.map(({ name, argv }) => ({ name, argv, ...to_result(argv) }))
	};
	return {
		filename: '../../crates/gro_cli/tests/fixtures/verb/grant_flags.json',
		content: JSON.stringify(fixture, null, '\t') + '\n'
	};
};
