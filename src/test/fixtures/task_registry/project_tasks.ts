import type * as clean_override_task from './clean_override.task_fixture.ts';
import type * as no_args_task from './no_args.task_fixture.ts';
import type * as required_args_task from './required_args.task_fixture.ts';
import type * as union_args_task from './union_args.task_fixture.ts';

// Augments `ProjectTasks` like a project's generated `gro_tasks.ts`, keyed by task paths,
// for the type-level tests in `task_registry.test.ts`.
// This applies to Gro's whole typecheck, so it uses `fixture/` paths,
// including one named like a builtin (`clean`), which never shadows it.
declare module '#lib/task_registry.ts' {
	interface ProjectTasks {
		'fixture/clean.task.ts': typeof clean_override_task;
		'fixture/no_args.task.ts': typeof no_args_task;
		'fixture/required_args.task.ts': typeof required_args_task;
		'fixture/union_args.task.ts': typeof union_args_task;
	}
}
