import type * as clean_override_task from './clean_override.task_fixture.ts';
import type * as no_args_task from './no_args.task_fixture.ts';
import type * as required_args_task from './required_args.task_fixture.ts';

// Augments `ProjectTasks` like a project's generated `gro_tasks.ts`,
// for the type-level tests in `task_registry.test.ts`.
// This applies to Gro's whole typecheck, so it shadows a builtin
// that Gro's own tasks never invoke (`clean`) and otherwise uses `fixture/` names.
declare module '#lib/task_registry.ts' {
	interface ProjectTasks {
		clean: typeof clean_override_task;
		'fixture/no_args': typeof no_args_task;
		'fixture/required_args': typeof required_args_task;
	}
}
