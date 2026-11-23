import { args_parse, type Args } from '@fuzdev/fuz_util/args.ts';
import type { Schema } from '@fuzdev/fuz_schema/schema.ts';

import type { Gen } from '#lib/gen.ts';
import type { BuiltinProgramGrant, BuiltinTaskGrants } from '#lib/builtin_host.ts';
import { fs } from '#lib/fs.ts';
import { create_fs_paths, type FsPaths } from '#lib/fs_paths.ts';
import { cwd } from '#lib/cwd.ts';
import { env } from '#lib/env.ts';
import { cli_grants } from '#lib/gro_builtin_tasks.gen.ts';
import { task_registry_find_builtins } from '#lib/task_registry.ts';
import { import_module_node } from '#lib/internal/node_import_module.ts';
import { posix_dirname, posix_join } from '#lib/internal/posix_path.ts';
import {
	merge_forwarded_args,
	to_forwarded_args,
	to_forwarded_args_by_command,
	to_grant_flags,
	to_raw_rest_args,
	to_task_args
} from '#lib/task_args.ts';
import { is_task_path, TASK_VERB, to_task_command } from '#lib/task.ts';
import { is_task_error } from '#lib/task_error.ts';

/*

Writes the fixture the binary's closure of a command is held to: for each case's CLI args,
the builtins the command runs, the union of their grants and the CLI's own row,
and why the command can't run on the binary, if it can't.
The closure is computed here, a reference with no consumer on Node, by the rules each step of
the Node Gro follows, and the results are written for the native closure's test to read,
never edited by hand, `gro gen --check` failing when the table or a rule moves.
It reads the table's sources, the builtin task modules and the CLI's row in the table's genfile,
not the generated table, which the same `gro gen` pass rewrites.

The rules, and where each comes from:

- The CLI's own row always, which `gro_main` cuts its host by before it parses the args.
- The command is the builtin `invoke_cli` invokes by name, after `to_grant_flags` and `to_task_args`;
  with none, the listing and `--version` hold only the CLI's row, as does `gro task`'s listing.
  `gro task <path>` runs a project task, whose grants no table holds.
- The command's args are what `run_task` validates: its direct args, the last `-- gro <name>`
  section's merged over them (`invoke_task`), parsed by the builtin's `Args` (`args_parse`),
  so `--no-build` after `--build` unsets it, as on Node.
- Each builtin's `invokes` joins the closure transitively. An invoke marked `when` joins only when
  the invoking builtin's parsed args hold `true` for it, `run_task`'s guard of the invoke;
  the args a builtin passes another are its code, so only the command's are read,
  and a `when` on a builtin another invokes throws here, as no args say whether it's set.
- An invoke marked `check` drops the writes to the project of the invoked builtin's grants,
  and of the builtins it invokes in turn (`BuiltinInvokeGrant`'s `check`); a builtin reached
  both ways is held both ways. The mark holds whatever the CLI forwards: on Node a section like
  `-- gro gen --no-check` reaches the invoked builtin, merged over the args its invoker passes,
  and runs it in write mode, a write the closure doesn't hold: the closure names each such builtin
  in `check_turned_off`, and the binary refuses the command line before anything runs.
- The command runs in check mode too, its own writes and its invokes' dropped the same way,
  when its parsed args hold `true` for `check` and the table marks it check-capable,
  some builtin's invoke of it marked `check`: `gen --check` and `format --check` write
  nothing on Node, so the closure holds no write for them.
- A builtin with no grants anywhere in the closure, `null` in the table, means the command runs
  on the Node Gro: nothing says what it reaches.
- `install` set in the command's args, with the closure holding the `npm_cli` program, means
  the same: the install needs the network. `sync` runs `npm install` only under its `install` arg,
  and `check` passes its own `install` to the `sync` it invokes.
- A command line Node refuses or answers with help before any builtin runs has no closure:
  `help` for a `--help` in the command's args, which prints its help, and `refused` naming why,
  for args the CLI refuses (`to_task_args`), a forwarded section that's malformed or asks for
  help (`invoke_task`), args the builtin's `Args` refuse (`run_task`), and a command that names
  no builtin (`invoke_cli`).

`fallback` is the closure's verdict alone. Whether a command runs on the Node Gro by policy,
like `clean`, or because the project's installed Gro is another version, is the binary's table,
not a rule here.

*/

interface ClosureCase {
	name: string;
	argv: Array<string>;
}

const CASES: Array<ClosureCase> = [
	{ name: 'listing', argv: [] },
	{ name: 'version', argv: ['--version'] },
	{ name: 'format_check', argv: ['format', '--check'] },
	{ name: 'gen_check', argv: ['gen', '--check'] },
	// a check-capable command's own `--check`, in each form Node reads it
	{ name: 'gen', argv: ['gen'] },
	{ name: 'format', argv: ['format'] },
	{ name: 'format_no_check', argv: ['format', '--no-check'] },
	{ name: 'gen_check_then_no_check', argv: ['gen', '--check', '--no-check'] },
	{ name: 'gen_no_check_then_check', argv: ['gen', '--no-check', '--check'] },
	{ name: 'gen_forwarded_check', argv: ['gen', '--', 'gro', 'gen', '--check'] },
	{
		name: 'gen_check_forwarded_no_check',
		argv: ['gen', '--check', '--', 'gro', 'gen', '--no-check']
	},
	{ name: 'format_check_valued', argv: ['format', '--check=true'] },
	{ name: 'gen_no_no_check', argv: ['gen', '--no-no-check'] },
	{ name: 'check', argv: ['check'] },
	{ name: 'check_build', argv: ['check', '--build'] },
	{ name: 'check_no_build', argv: ['check', '--no-build'] },
	{ name: 'check_build_then_no_build', argv: ['check', '--build', '--no-build'] },
	{ name: 'check_no_build_then_build', argv: ['check', '--no-build', '--build'] },
	{ name: 'check_install', argv: ['check', '--install'] },
	{ name: 'check_install_build', argv: ['check', '--install', '--build'] },
	{ name: 'check_forwarded_build', argv: ['check', '--', 'gro', 'check', '--build'] },
	{
		name: 'check_forwarded_no_build',
		argv: ['check', '--build', '--', 'gro', 'check', '--no-build']
	},
	{ name: 'check_no_no_build', argv: ['check', '--no-no-build'] },
	{
		name: 'check_forwarded_last_section_no_build',
		argv: ['check', '--', 'gro', 'check', '--build', '--', 'gro', 'check', '--no-build']
	},
	{
		name: 'check_forwarded_last_section_build',
		argv: ['check', '--', 'gro', 'check', '--no-build', '--', 'gro', 'check', '--build']
	},
	{ name: 'check_forwarded_other_section', argv: ['check', '--', 'vitest', '--build'] },
	{ name: 'check_forwarded_gen_write', argv: ['check', '--', 'gro', 'gen', '--no-check'] },
	{ name: 'check_forwarded_format_write', argv: ['check', '--', 'gro', 'format', '--no-check'] },
	{
		name: 'check_forwarded_gen_write_then_check',
		argv: ['check', '--', 'gro', 'gen', '--no-check', '--', 'gro', 'gen']
	},
	{ name: 'check_forwarded_gen_valued', argv: ['check', '--', 'gro', 'gen', '--check=1'] },
	{ name: 'check_forwarded_sync_install', argv: ['check', '--', 'gro', 'sync', '--install'] },
	{ name: 'check_grant_flags', argv: ['check', '--allow-read=.', '-A', '--build'] },
	{ name: 'check_workspace', argv: ['check', '--workspace'] },
	{ name: 'sync', argv: ['sync'] },
	{ name: 'sync_install', argv: ['sync', '--install'] },
	{ name: 'sync_forwarded_install', argv: ['sync', '--', 'gro', 'sync', '--install'] },
	{ name: 'test', argv: ['test'] },
	{ name: 'typecheck', argv: ['typecheck'] },
	{ name: 'lint', argv: ['lint'] },
	{ name: 'clean', argv: ['clean'] },
	{ name: 'release', argv: ['release'] },
	{ name: 'build', argv: ['build'] },
	{ name: 'task_listing', argv: ['task'] },
	{ name: 'task_path', argv: ['task', 'src/lib/x.task.ts'] },
	{ name: 'run', argv: ['run', 'x.ts'] },
	// Node runs no builtin for these
	{ name: 'check_help', argv: ['check', '--help'] },
	{ name: 'check_forwarded_help', argv: ['check', '--', 'gro', 'check', '--help'] },
	{ name: 'check_forwarded_malformed', argv: ['check', '--', 'gro'] },
	{ name: 'check_build_valued', argv: ['check', '--build=true'] },
	{ name: 'check_build_spaced_value', argv: ['check', '--build', 'false'] },
	{ name: 'check_short_flag', argv: ['check', '-h'] },
	{ name: 'unknown_command', argv: ['nope'] },
	// a flag keyed `_` reaches the positionals that name the command
	{ name: 'underscore_value_command', argv: ['--_=check', '--build'] },
	{ name: 'underscore_spaced_command', argv: ['--_', 'check', '--install'] },
	{ name: 'underscore_forwarded_command', argv: ['check', '--', 'gro', '--_=check', '--build'] },
	{ name: 'underscore_replaced', argv: ['check', '-_'] },
	{
		name: 'underscore_replaced_section',
		argv: ['check', '--', 'gro', '--no-_', 'check', '--build']
	},
	{ name: 'path_without_task', argv: ['src/lib/x.task.ts'] }
];

/**
 * Why Node runs no builtin for a command line, refusing it before any runs.
 */
type Refused = 'task_args' | 'forwarded_args' | 'forwarded_help' | 'invalid_args' | 'not_a_builtin';

/**
 * Why a command can't run on the binary.
 */
type Fallback =
	{ kind: 'ungranted'; builtin: string } | { kind: 'install' } | { kind: 'project_task' };

interface ClosureBuiltin {
	name: string;
	/**
	 * Whether the closure holds it in check mode, without its writes to the project.
	 */
	check: boolean;
}

interface ClosureProgram extends BuiltinProgramGrant {
	role: string;
}

interface Closure {
	outcome: 'runs';
	builtins: Array<ClosureBuiltin>;
	grants: {
		fs: { read: Array<string>; write: Array<string> };
		env: Array<string>;
		programs: Array<ClosureProgram>;
		project_modules: boolean;
		stdout: boolean;
		stderr: boolean;
	};
	fallback: Array<Fallback>;
	/**
	 * The builtins an invoke marked `check` runs that Node runs writing,
	 * a forwarded section's `check: false` merged over the invoke's `check: true`.
	 */
	check_turned_off: Array<string>;
}

/**
 * What Node does with a command line: runs the closure's builtins, prints a builtin's help,
 * or refuses it before any builtin runs.
 */
type Outcome = Closure | { outcome: 'help' } | { outcome: 'refused'; refused: Refused };

/**
 * The command's args as `invoke_task` and `run_task` read them, or why they run nothing.
 */
type CommandArgs =
	{ outcome: 'runs'; args: Args } | { outcome: 'help' } | { outcome: 'refused'; refused: Refused };

/**
 * A builtin as the closure reads it: its grants and the schema its args are parsed by.
 */
interface ClosureTableEntry {
	grants: BuiltinTaskGrants | null;
	Args: Schema<Args, Args> | undefined;
}

/**
 * The table's sources: the CLI's row and each builtin by name.
 */
interface ClosureTable {
	cli: BuiltinTaskGrants;
	builtins: Map<string, ClosureTableEntry>;
}

const sorted = (values: Iterable<string>): Array<string> => [...new Set(values)].sort();

// by code unit, as `sort` does with no compare, so no locale decides the order
const compare = (a: string, b: string): number => (a < b ? -1 : a > b ? 1 : 0);

/**
 * The parsed args of the builtin `name` run as the command, in the order `invoke_task`
 * and `run_task` read them: the forwarded section, refused when malformed or asking for help,
 * then the merged args' `--help`, then the builtin's `Args`.
 */
const to_command_args = async (
	name: string,
	{ Args }: ClosureTableEntry,
	args: Args,
	raw_rest_args: Array<string>,
	fs_paths: FsPaths
): Promise<CommandArgs> => {
	let forwarded: Args;
	try {
		forwarded = await to_forwarded_args(to_task_command(name), raw_rest_args, fs_paths);
	} catch (error) {
		if (!is_task_error(error)) throw error;
		return { outcome: 'refused', refused: 'forwarded_args' };
	}
	if (forwarded.help) return { outcome: 'refused', refused: 'forwarded_help' };
	const merged = merge_forwarded_args(args, forwarded);
	if (merged.help) return { outcome: 'help' };
	if (!Args) return { outcome: 'runs', args: merged };
	const parsed = args_parse(merged, Args);
	if (!parsed.ok) return { outcome: 'refused', refused: 'invalid_args' };
	return { outcome: 'runs', args: parsed.value };
};

// the program's fields in one order, so two builtins granting it alike with their keys
// in different orders are held once
const to_program_key = ({
	role,
	name,
	local,
	pass_env,
	omit_env,
	env: values
}: ClosureProgram): string => JSON.stringify([role, name, local, pass_env, omit_env, values]);

/**
 * The closure of the command `argv`, the args after the CLI's own path.
 */
const to_closure = async (
	argv: Array<string>,
	table: ClosureTable,
	fs_paths: FsPaths
): Promise<Outcome> => {
	const builtins: Array<ClosureBuiltin> = [];
	const fallback: Array<Fallback> = [];
	const read: Array<string> = [];
	const write: Array<string> = [];
	const env_names: Array<string> = [];
	const programs: Map<string, ClosureProgram> = new Map();
	let project_modules = false;
	let stdout = false;
	let stderr = false;
	const add = (grants: BuiltinTaskGrants, check: boolean): void => {
		read.push(...(grants.fs?.read ?? []));
		if (!check) write.push(...(grants.fs?.write ?? []));
		env_names.push(...(grants.env ?? []));
		for (const [role, grant] of Object.entries(grants.programs ?? {})) {
			const program = { role, ...grant };
			// a role two builtins grant differently is held each way
			programs.set(to_program_key(program), program);
		}
		project_modules ||= grants.project_modules === true;
		stdout ||= grants.stdout === true;
		stderr ||= grants.stderr === true;
	};
	const check_turned_off: Array<string> = [];
	// the forwarded sections by command, read once the command's args parsed, so none is malformed
	let forwarded_by_command: Record<string, Args | undefined> = {};
	const visited: Set<string> = new Set();
	// `args` is the builtin's parsed args when it's the command, `null` when another invokes it
	const visit = (name: string, args: Args | null, check: boolean): void => {
		const key = `${name}:${check}`;
		if (visited.has(key)) return;
		visited.add(key);
		const entry = table.builtins.get(name);
		if (!entry) throw Error(`\`${name}\` is no builtin`);
		builtins.push({ name, check });
		const { grants } = entry;
		if (grants === null) {
			// reached in both modes, it's one fallback
			if (!builtins.some((b) => b.name === name && b.check !== check)) {
				fallback.push({ kind: 'ungranted', builtin: name });
			}
			return;
		}
		add(grants, check);
		for (const [invoked, invoke] of Object.entries(grants.invokes ?? {})) {
			const { when } = invoke;
			if (when !== undefined) {
				if (args === null) {
					throw Error(
						`\`${name}\` invokes \`${invoked}\` when its \`${when}\` arg is set, ` +
							`and the closure reads only the command's args`
					);
				}
				if (!(Object.hasOwn(args, when) && args[when] === true)) continue;
			}
			const forwarded = forwarded_by_command[to_task_command(invoked)];
			if (invoke.check === true && forwarded) {
				// what `invoke_task` runs it with, parsed as `run_task` parses it: invalid args run nothing
				const invoked_args = merge_forwarded_args({ check: true }, forwarded);
				const Args = table.builtins.get(invoked)?.Args;
				const parsed = Args ? args_parse(invoked_args, Args) : { ok: true, value: invoked_args };
				if (parsed.ok && parsed.value.check !== true) check_turned_off.push(invoked);
			}
			visit(invoked, null, check || invoke.check === true);
		}
	};

	add(table.cli, false);
	const { cli_args } = to_grant_flags(argv);
	let task_args;
	try {
		task_args = to_task_args(cli_args);
	} catch (error) {
		if (!is_task_error(error)) throw error;
		return { outcome: 'refused', refused: 'task_args' };
	}
	const { task_name, task_path, args } = task_args;
	if (task_name === TASK_VERB) {
		if (task_path !== null && is_task_path(task_path)) fallback.push({ kind: 'project_task' });
	} else if (task_name) {
		const entry = table.builtins.get(task_name);
		if (!entry) return { outcome: 'refused', refused: 'not_a_builtin' };
		const command = await to_command_args(
			task_name,
			entry,
			args,
			to_raw_rest_args(cli_args),
			fs_paths
		);
		if (command.outcome !== 'runs') return command;
		const command_args = command.args;
		forwarded_by_command = await to_forwarded_args_by_command(to_raw_rest_args(cli_args), fs_paths);
		// check-capable: some builtin's invoke of it is marked `check`
		const check_capable = [...table.builtins.values()].some(
			({ grants }) => grants?.invokes?.[task_name as keyof typeof grants.invokes]?.check === true
		);
		const check_mode =
			check_capable && Object.hasOwn(command_args, 'check') && command_args.check === true;
		visit(task_name, command_args, check_mode);
		if (
			Object.hasOwn(command_args, 'install') &&
			command_args.install === true &&
			[...programs.values()].some(({ role }) => role === 'npm_cli')
		) {
			fallback.push({ kind: 'install' });
		}
	}

	return {
		outcome: 'runs',
		builtins: builtins.sort((a, b) => compare(a.name, b.name) || Number(a.check) - Number(b.check)),
		grants: {
			fs: { read: sorted(read), write: sorted(write) },
			env: sorted(env_names),
			programs: [...programs.values()].sort(
				(a, b) => compare(a.role, b.role) || compare(JSON.stringify(a), JSON.stringify(b))
			),
			project_modules,
			stdout,
			stderr
		},
		// by kind, then builtin, an order no walk decides
		fallback: fallback.sort((a, b) => compare(JSON.stringify(a), JSON.stringify(b))),
		check_turned_off: sorted(check_turned_off)
	};
};

/**
 * Reads the table's sources: the CLI's row, and each builtin task module in `src/lib`.
 */
const load_table = async (fs_paths: FsPaths, src_lib_path: string): Promise<ClosureTable> => {
	const opened = await fs_paths.open_dir_path(src_lib_path);
	if (!opened) throw Error(`no directory at ${src_lib_path}`);
	const src_lib = opened.dir.dir(opened.path).readonly();
	const builtins: Map<string, ClosureTableEntry> = new Map();
	const entries = await task_registry_find_builtins(
		src_lib,
		import_module_node,
		fs_paths.project.path
	);
	for (const { name, id, grants } of entries) {
		const { task } = await import_module_node(id);
		builtins.set(name, { grants, Args: task.Args });
	}
	return { cli: cli_grants, builtins };
};

export const gen: Gen = async ({ origin_id, origin_path }) => {
	const fs_paths = create_fs_paths(fs, { cwd: cwd(), home: env.get('HOME') });
	// Gro's `src/lib/`, beside this genfile's `src/test/`
	const table = await load_table(fs_paths, posix_join(posix_dirname(origin_id), '../lib'));
	const names: Set<string> = new Set();
	const cases = [];
	for (const { name, argv } of CASES) {
		if (names.has(name)) throw Error(`two closure cases are named ${name}`);
		names.add(name);
		cases.push({ name, argv, ...(await to_closure(argv, table, fs_paths)) });
	}
	const fixture = {
		description:
			"Each case's CLI args, the args after the CLI's own path, and what Node does with them: " +
			"`runs` with the command's closure, the builtins it runs, each in check mode or not, " +
			"the union of their grants and the CLI's own row, and the closure's own verdict on running " +
			'on the Node Gro, its `fallback`, apart from any policy of the binary; ' +
			'`help` for a builtin printing its help; or `refused`, naming why Node refuses the command line ' +
			'before any builtin runs. ' +
			`Generated by ${origin_path}, never edited by hand.`,
		cases
	};
	return {
		filename: '../../crates/gro_cli/tests/fixtures/verb/command_closure.json',
		content: JSON.stringify(fixture, null, '\t') + '\n'
	};
};
