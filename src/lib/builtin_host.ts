import type { Args } from '@fuzdev/fuz_util/args.ts';
import type { Schema } from '@fuzdev/fuz_schema/schema.ts';

import type { Env } from './env.ts';
import type { FsPaths } from './fs_paths.ts';
import type { Spawn, SpawnProgram, SpawnProgramOptions } from './spawn.ts';
import type { GroBuiltinTasks } from './gro_builtin_tasks.ts';
import type { ImportModule } from './modules.ts';
import type { InvokeTask, InvokeTaskRest, TaskContext } from './task.ts';
import { SpawnError } from './spawn_error.ts';
import { create_record } from './internal/records.ts';

/*

What a builtin task holds comes from the root, never from a host module the task imports,
since Gro's tasks are exported and a granted import would lend its grant to every importer.
Each builtin declares its `task.grants`, the summaries genfile emits them as `GRO_BUILTIN_TASK_GRANTS`,
and the root's handles are cut by them into the builtin's `host`.
The root's project loader is the one every task and the project's modules load through,
so a builtin that loads the project's modules takes it from its `host` rather than importing Node's.

On the Node Gro the cut is a review signal, as a narrowed root is, not an enforced boundary:
`env` is narrowed and `programs` refuses a name the grants don't list,
but `fs_paths` is the root's whole `FsPaths`, and any module can still import a host module.
`Object.freeze` and `Object.hasOwn` are copied when this module loads, and the host and its
`programs` are null-prototype records, but the grants are read by plain property access and the
args are copied through the array iterator, so code in the same realm that patches intrinsics
or the grants isn't held by the cut.

*/

const { freeze, hasOwn } = Object;

/**
 * A program a builtin starts, keyed in its grants by the name `spawn.program` resolves,
 * and how the root builds its environment.
 */
export interface BuiltinProgramGrant {
	/**
	 * The host variables the program sees beyond the base set, as `SpawnProgramOptions`.
	 */
	readonly pass_env?: ReadonlyArray<string> | 'all';
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
	 * Whether the task loads the project's modules, like genfiles, which run as project code:
	 * through the root's project loader, `host.import_module`,
	 * invoking tasks as a project task does, through `host.invoke_project_task`,
	 * which `invokes` doesn't check, so the task reaches whatever a project task does,
	 * every builtin's grants included.
	 */
	readonly project_modules?: true;
	/**
	 * The builtins the task invokes, each run with its own grants;
	 * invoking any other task through its own `invoke_task` is an error,
	 * though with `project_modules` the modules it loads invoke any task, unchecked.
	 */
	readonly invokes?: ReadonlyArray<keyof GroBuiltinTasks>;
}

/**
 * The root's handles, which `invoke_task` carries to every task it runs
 * and cuts by a builtin's grants into its `host`; no task's context holds them.
 */
export interface BuiltinHostRoot {
	readonly env: Env;
	readonly spawn: Spawn;
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
	/**
	 * The `invoke_task` a project task's context has, unchecked by the builtin's `invokes`,
	 * held only with the `project_modules` grant, for the project's modules it loads.
	 */
	readonly invoke_project_task: InvokeTask;
	/**
	 * The CLI args from the first `--` on, copied and frozen for each host.
	 */
	readonly raw_rest_args: ReadonlyArray<string>;
}

/**
 * The programs a builtin's grants name, each built by the root's `spawn` with the grant's
 * environment, as `Spawn`'s `program` and `find` resolve them.
 * A name the grants don't list throws `SpawnError` `not_granted`.
 */
export interface BuiltinPrograms<TName extends string = string> {
	/**
	 * @throws `SpawnError` - `not_found` and the rest `Spawn.program` throws, or `not_granted`
	 */
	program(name: TName): SpawnProgram;
	/**
	 * @returns the program, or `null` when it isn't found
	 * @throws `SpawnError` - as `Spawn.find` does, or `not_granted`
	 */
	find(name: TName): SpawnProgram | null;
}

/**
 * The handles a builtin task gets, each present only when its grants name it.
 */
export type BuiltinHost<TGrants extends BuiltinTaskGrants = BuiltinTaskGrants> = {
	/**
	 * The CLI args from the first `--` on, as `invoke_task` was given them.
	 */
	readonly raw_rest_args: ReadonlyArray<string>;
} & (TGrants extends { fs: BuiltinFsGrant } ? { readonly fs_paths: FsPaths } : unknown) &
	(TGrants extends { env: ReadonlyArray<string> } ? { readonly env: Env } : unknown) &
	(TGrants extends { project_modules: true }
		? { readonly import_module: ImportModule; readonly invoke_project_task: InvokeTask }
		: unknown) &
	(TGrants extends { programs: infer TPrograms }
		? { readonly programs: BuiltinPrograms<Extract<keyof TPrograms, string>> }
		: unknown);

/**
 * The builtins a builtin's grants let it invoke, `never` when they name none.
 */
export type BuiltinInvokes<TGrants extends BuiltinTaskGrants> = TGrants extends {
	invokes: ReadonlyArray<infer TName extends keyof GroBuiltinTasks>;
}
	? TName
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
	const host: Record<string, unknown> = create_record({
		raw_rest_args: freeze([...invocation.raw_rest_args])
	});
	if (grants.fs) host.fs_paths = invocation.fs_paths;
	if (grants.env) host.env = root.env.narrow(grants.env);
	if (grants.project_modules) {
		host.import_module = invocation.import_module;
		host.invoke_project_task = invocation.invoke_project_task;
	}
	const { programs } = grants;
	if (programs) {
		const { spawn } = root;
		const to_options = (name: string): SpawnProgramOptions | undefined => {
			const grant = hasOwn(programs, name) ? programs[name] : undefined;
			if (!grant) throw new SpawnError('not_granted', name);
			return grant.pass_env === undefined ? undefined : { pass_env: grant.pass_env };
		};
		host.programs = freeze(
			create_record({
				program: (name: string): SpawnProgram => spawn.program(name, to_options(name)),
				find: (name: string): SpawnProgram | null => spawn.find(name, to_options(name))
			})
		);
	}
	return freeze(host) as BuiltinHost<TGrants>;
};
