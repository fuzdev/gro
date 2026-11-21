import type { Args } from '@fuzdev/fuz_util/args.ts';
import type { Schema } from '@fuzdev/fuz_schema/schema.ts';

import type { Env } from './env.ts';
import type { FsPaths } from './fs_paths.ts';
import type { Spawn, SpawnProgram, SpawnProgramOptions } from './spawn.ts';
import type { ProcessOutput } from './stdout.ts';
import type { GroBuiltinTasks } from './gro_builtin_tasks.ts';
import type { ImportModule } from './modules.ts';
import type { InvokeTaskRest, TaskContext } from './task.ts';
import { SpawnError } from './spawn_error.ts';
import { find_local_cli, to_local_cli_path } from './cli.ts';
import { create_record } from './internal/records.ts';

/*

What a builtin task holds comes from the root, never from a host module the task imports,
since Gro's tasks are exported and a granted import would lend its grant to every importer.
Each builtin declares its `task.grants`, the summaries genfile emits them as `GRO_BUILTIN_TASK_GRANTS`,
and the root's handles are cut by them into the builtin's `host`.
The root's project loader is the one every task and the project's modules load through,
so a builtin that loads the project's modules takes it from its `host` rather than importing Node's.

On the Node Gro the cut is a review signal, as a narrowed root is, not an enforced boundary:
`env` is narrowed and `programs` refuses a role the grants don't list,
but `fs_paths` is the root's whole `FsPaths`, and any module can still import a host module.
`Object.freeze` and `Object.hasOwn` are copied when this module loads, the host and its
`programs` are null-prototype records, and `GRO_BUILTIN_TASK_GRANTS` is deep-frozen with
null-prototype objects, so a property added to `Object.prototype` widens no host:
a guardrail against accidental prototype pollution, not against a module built to defeat it,
and an `invoke_task` of a builtin lends its grants by design.

*/

const { freeze, hasOwn } = Object;

/**
 * A program a builtin starts, keyed in its grants by its role: the program's name, how it's found,
 * and how the root builds its environment, as `SpawnProgramOptions` has it.
 * A role is its tool's name with `_cli` (`eslint_cli`, `vitest_cli`, `git_cli`),
 * the tool's dashes dropped or made underscores (`sveltekit_cli`, `svelte_check_cli`),
 * and the tool is the product when its program is named otherwise (`typescript_cli` runs `tsc`);
 * the program is always the one its grant names.
 * Data a runtime reads from the table without importing the task.
 */
export interface BuiltinProgramGrant {
	/**
	 * The program, as `spawn.program` resolves it.
	 */
	readonly name: string;
	/**
	 * Whether the program is the project's own install, `node_modules/.bin/<name>`,
	 * never one on `PATH`, as `find_local_cli` looks.
	 */
	readonly local?: true;
	/**
	 * The host variables the program sees beyond the base set.
	 */
	readonly pass_env?: ReadonlyArray<string> | 'all';
	/**
	 * The variables left out of what `pass_env` passes.
	 */
	readonly omit_env?: ReadonlyArray<string>;
	/**
	 * The variables set for the program, whatever the host's are.
	 */
	readonly env?: Readonly<Record<string, string>>;
}

/**
 * The filesystem a builtin reaches, each path relative to the project root, `.` the project.
 */
export interface BuiltinFsGrant {
	readonly read?: ReadonlyArray<string>;
	readonly write?: ReadonlyArray<string>;
}

/**
 * What a builtin task needs, the data its `host` is cut by,
 * declared as the builtin's `task.grants` and emitted as `GRO_BUILTIN_TASK_GRANTS`,
 * which a runtime reads without importing the task.
 */
export interface BuiltinTaskGrants {
	readonly fs?: BuiltinFsGrant;
	/**
	 * The environment variables the task reads, exact names.
	 */
	readonly env?: ReadonlyArray<string>;
	readonly programs?: Readonly<Record<string, BuiltinProgramGrant>>;
	/**
	 * Whether the task loads the project's modules, like genfiles, which run as project code,
	 * through the root's project loader, `host.import_module`.
	 */
	readonly project_modules?: true;
	/**
	 * Whether the task writes to standard output, `host.stdout`.
	 */
	readonly stdout?: true;
	/**
	 * Whether the task writes to standard error, `host.stderr`.
	 */
	readonly stderr?: true;
	/**
	 * The builtins the task invokes, by name, each run with its own grants,
	 * marked with what narrows the invoke;
	 * invoking any other task through its own `invoke_task` is an error.
	 */
	readonly invokes?: Readonly<Partial<Record<keyof GroBuiltinTasks, BuiltinInvokeGrant>>>;
}

/**
 * How a builtin invokes another, data a runtime reads to know which builtins
 * a command's closure holds, and with what grants, from its command line.
 * `{}` marks nothing: the invoke may run whatever the invoking task's args are.
 */
export interface BuiltinInvokeGrant {
	/**
	 * A boolean arg of the invoking task that defaults to `false`, the invoke running only
	 * when it's `true`, so the closure holds the invoked builtin only when the command's args set it,
	 * like `check`'s `build` under `--build`;
	 * an invoke when it's not is an error.
	 */
	readonly when?: string;
}

/**
 * The root's handles, which `invoke_task` carries to every task it runs
 * and cuts by a builtin's grants into its `host`; no task's context holds them.
 */
export interface BuiltinHostRoot {
	readonly env: Env;
	readonly spawn: Spawn;
	readonly stdout: ProcessOutput;
	readonly stderr: ProcessOutput;
}

/**
 * What `invoke_task` passes to the task it runs beside the root's handles,
 * the parts of it a builtin's `host` holds when its grants name them.
 */
export interface BuiltinHostInvocation {
	/**
	 * The root's boundary, held only with an `fs` grant.
	 */
	readonly fs_paths: FsPaths;
	/**
	 * The root's project loader, held only with the `project_modules` grant.
	 */
	readonly import_module: ImportModule;
}

/**
 * The programs a builtin's grants name, by role, each the grant's `name` built by the root's `spawn`
 * with the grant's environment, as `Spawn`'s `program` and `find` resolve them,
 * only in the project's `node_modules/.bin` for a grant that's `local`, as `find_local_cli` looks.
 * A role the grants don't list throws `SpawnError` `not_granted`.
 */
export interface BuiltinPrograms<TRole extends string = string> {
	/**
	 * @param role - the grant's key
	 * @throws `SpawnError` - `not_found` and the rest `Spawn.program` throws, or `not_granted`
	 */
	program(role: TRole): SpawnProgram;
	/**
	 * @param role - the grant's key
	 * @returns the program, or `null` when it isn't found
	 * @throws `SpawnError` - as `Spawn.find` does, or `not_granted`
	 * @throws `TaskError` - for a `local` grant, as `find_local_cli` throws
	 */
	find(role: TRole): SpawnProgram | null;
}

/**
 * The handles a builtin task gets, each present only when its grants name it.
 */
export type BuiltinHost<TGrants extends BuiltinTaskGrants = BuiltinTaskGrants> = object &
	(TGrants extends { fs: BuiltinFsGrant } ? { readonly fs_paths: FsPaths } : unknown) &
	(TGrants extends { env: ReadonlyArray<string> } ? { readonly env: Env } : unknown) &
	(TGrants extends { stdout: true } ? { readonly stdout: ProcessOutput } : unknown) &
	(TGrants extends { stderr: true } ? { readonly stderr: ProcessOutput } : unknown) &
	(TGrants extends { project_modules: true } ? { readonly import_module: ImportModule } : unknown) &
	(TGrants extends { programs: infer TPrograms }
		? { readonly programs: BuiltinPrograms<Extract<keyof TPrograms, string>> }
		: unknown);

/**
 * The builtins a builtin's grants let it invoke, `never` when they name none.
 */
export type BuiltinInvokes<TGrants extends BuiltinTaskGrants> = TGrants extends {
	invokes: infer TInvokes;
}
	? Extract<keyof TInvokes, keyof GroBuiltinTasks>
	: never;

/**
 * A builtin's `invoke_task`, typed to the builtins its grants' `invokes` name,
 * each taking its own `Args`, so invoking any other fails to compile.
 */
export type BuiltinInvokeTask<TName extends keyof GroBuiltinTasks> = <TInvoked extends TName>(
	task_name: TInvoked,
	...rest: InvokeTaskRest<TInvoked>
) => Promise<void>;

/**
 * A builtin task's context: a project task's, its `invoke_task` typed by its grants' `invokes`,
 * and the `host` cut by its grants.
 */
export interface BuiltinTaskContext<
	TArgs = object,
	TGrants extends BuiltinTaskGrants = BuiltinTaskGrants
> extends Omit<TaskContext<TArgs>, 'invoke_task'> {
	invoke_task: BuiltinInvokeTask<BuiltinInvokes<TGrants>>;
	host: BuiltinHost<TGrants>;
}

/**
 * A builtin task, a `Task` with the `grants` its `host` is cut by,
 * so a module re-exporting the `task` carries them.
 */
export interface BuiltinTask<TArgs = Args, TGrants extends BuiltinTaskGrants = BuiltinTaskGrants> {
	run: (ctx: BuiltinTaskContext<TArgs, TGrants>) => unknown;
	summary?: string;
	Args?: Schema<Args, Args>;
	grants: TGrants;
}

/**
 * Cuts the root's handles by a builtin's grants into its `host`, a frozen null-prototype record.
 * @param root - the root's handles
 * @param grants - the builtin's grants
 * @param invocation - what `invoke_task` passes, each part held only when the grants name it
 */
export const create_builtin_host = <TGrants extends BuiltinTaskGrants>(
	root: BuiltinHostRoot,
	grants: TGrants,
	invocation: BuiltinHostInvocation
): BuiltinHost<TGrants> => {
	const host: Record<string, unknown> = create_record({});
	if (grants.fs) host.fs_paths = invocation.fs_paths;
	if (grants.env) host.env = root.env.narrow(grants.env);
	if (grants.stdout) host.stdout = root.stdout;
	if (grants.stderr) host.stderr = root.stderr;
	if (grants.project_modules) host.import_module = invocation.import_module;
	const { programs } = grants;
	if (programs) {
		const { spawn } = root;
		// the grant of `role` and the program's environment, refusing a role it doesn't grant
		const to_granted = (
			role: string
		): { grant: BuiltinProgramGrant; options: SpawnProgramOptions } => {
			const grant = hasOwn(programs, role) ? programs[role] : undefined;
			if (!grant) throw new SpawnError('not_granted', role);
			const options: SpawnProgramOptions = {};
			if (grant.pass_env !== undefined) options.pass_env = grant.pass_env;
			if (grant.omit_env !== undefined) options.omit_env = grant.omit_env;
			if (grant.env !== undefined) options.env = grant.env;
			return { grant, options };
		};
		host.programs = freeze(
			create_record({
				program: (role: string): SpawnProgram => {
					const { grant, options } = to_granted(role);
					return spawn.program(grant.local ? to_local_cli_path(grant.name) : grant.name, options);
				},
				find: (role: string): SpawnProgram | null => {
					const { grant, options } = to_granted(role);
					return grant.local
						? find_local_cli(spawn, grant.name, options)
						: spawn.find(grant.name, options);
				}
			})
		);
	}
	return freeze(host) as BuiltinHost<TGrants>;
};
