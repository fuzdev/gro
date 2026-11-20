import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';

import { has_sveltekit_library, has_sveltekit_app } from './sveltekit_helpers.ts';
import { package_json_load } from './package_json.ts';
import type { FsPaths } from './fs_paths.ts';
import type {
	BuiltinInvokes,
	BuiltinInvokeTask,
	BuiltinTask,
	BuiltinTaskGrants
} from './builtin_host.ts';

/** @nodocs */
export const Args = s.object({
	dry: s.with_default(
		meta(s.boolean, { description: 'build and prepare without actually publishing or deploying' }),
		false
	),
	check: s.with_default(meta(s.boolean, { description: 'dual of no-check' }), true),
	'no-check': s.with_default(
		meta(s.boolean, { description: 'opt out of checking before publishing' }),
		false
	),
	build: s.with_default(meta(s.boolean, { description: 'dual of no-build' }), true),
	'no-build': s.with_default(meta(s.boolean, { description: 'opt out of building' }), false),
	pull: s.with_default(meta(s.boolean, { description: 'dual of no-pull' }), true),
	'no-pull': s.with_default(meta(s.boolean, { description: 'opt out of git pull' }), false),
	sync: s.with_default(meta(s.boolean, { description: 'dual of no-sync' }), true),
	'no-sync': s.with_default(meta(s.boolean, { description: 'opt out of gro sync' }), false),
	install: s.with_default(meta(s.boolean, { description: 'dual of no-install' }), true),
	'no-install': s.with_default(
		meta(s.boolean, { description: 'opt out of installing packages' }),
		false
	),
	gen: s.with_default(meta(s.boolean, { description: 'dual of no-gen' }), true),
	'no-gen': s.with_default(
		meta(s.boolean, { description: 'opt out of gro gen in deploy build' }),
		false
	),
	force_build: s.with_default(
		meta(s.boolean, { description: 'force a fresh build, ignoring the cache' }),
		false
	)
});
export type Args = s.Infer<typeof Args>;

const grants = {
	// reads `package.json` and what tells a library or an app
	fs: { read: ['.'] },
	invokes: ['publish', 'deploy']
} as const satisfies BuiltinTaskGrants;

/** @nodocs */
export const task: BuiltinTask<Args, typeof grants> = {
	summary: 'publish and deploy',
	Args,
	grants,
	run: ({ task_args, invoke_task, host }) =>
		run_release(task_args, { fs_paths: host.fs_paths, invoke_task })
};

/**
 * What `run_release` reaches, picked from a builtin's `host` and context.
 */
export interface ReleaseDeps {
	fs_paths: FsPaths;
	invoke_task: BuiltinInvokeTask<BuiltinInvokes<typeof grants>>;
}

/**
 * Publishes a library and deploys an app, `gro release`'s body.
 * @param args - the task's parsed args
 * @param deps - the project's boundary and the nested tasks
 */
export const run_release = async (args: Args, deps: ReleaseDeps): Promise<void> => {
	const { dry, check, build, pull, sync, install, gen, force_build } = args;
	const { fs_paths, invoke_task } = deps;

	const package_json = await package_json_load(fs_paths);

	const publish = (await has_sveltekit_library(fs_paths, package_json)).ok;
	if (publish) {
		await invoke_task('publish', {
			optional: true,
			dry,
			check,
			build,
			pull,
			sync,
			install,
			force_build
		});
	}
	if (has_sveltekit_app(package_json).ok) {
		await invoke_task('deploy', {
			build: build && !publish,
			dry,
			pull,
			sync,
			install,
			gen,
			force_build
		});
	}
};
