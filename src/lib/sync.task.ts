import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';
import type { Logger } from '@fuzdev/fuz_util/log.ts';

import { package_json_sync } from './package_json.ts';
import { project_has_sveltekit_app, sveltekit_sync } from './sveltekit_helpers.ts';
import {
	install_with_cache_healing_or_throw,
	NPM_CLI_PROGRAM_GRANT
} from './npm_install_helpers.ts';
import { SVELTEKIT_CLI } from './constants.ts';
import type { FsPaths } from './fs_paths.ts';
import type { GroConfig } from './gro_config.ts';
import type { ProcessOutput } from './stdout.ts';
import type {
	BuiltinInvokes,
	BuiltinInvokeTask,
	BuiltinPrograms,
	BuiltinTask,
	BuiltinTaskGrants
} from './builtin_host.ts';

/** @nodocs */
export const Args = s.object({
	sveltekit: s.with_default(meta(s.boolean, { description: 'dual of no-sveltekit' }), true),
	'no-sveltekit': s.with_default(
		meta(s.boolean, { description: 'opt out of svelte-kit sync' }),
		false
	),
	package_json: s.with_default(meta(s.boolean, { description: 'dual of no-package_json' }), true),
	'no-package_json': s.with_default(
		meta(s.boolean, { description: 'opt out of package.json sync' }),
		false
	),
	gen: s.with_default(meta(s.boolean, { description: 'dual of no-gen' }), true),
	'no-gen': s.with_default(meta(s.boolean, { description: 'opt out of running gen' }), false),
	install: s.with_default(meta(s.boolean, { description: 'opt into installing packages' }), false)
});
export type Args = s.Infer<typeof Args>;

const grants = {
	// reads `package.json` and Gro's installed packages, and writes `package.json`'s exports
	fs: { read: ['.'], write: ['.'] },
	programs: {
		// `--install`
		npm_cli: NPM_CLI_PROGRAM_GRANT,
		// `svelte-kit sync` loads the project's Vite config, code whose environment Gro can't know
		sveltekit_cli: { name: SVELTEKIT_CLI, local_first: true, pass_env: 'all' }
	},
	// the install's output
	stderr: true,
	invokes: ['gen']
} as const satisfies BuiltinTaskGrants;

/** @nodocs */
export const task: BuiltinTask<Args, typeof grants> = {
	summary: 'optionally install packages, sync SvelteKit, update `package.json`, and run `gro gen`',
	Args,
	grants,
	run: ({ task_args, invoke_task, config, log, host }) =>
		run_sync(
			task_args,
			{ map_package_json: config.map_package_json },
			{ fs_paths: host.fs_paths, programs: host.programs, stderr: host.stderr, invoke_task, log }
		)
};

/**
 * The data `run_sync` takes beside its args.
 */
export interface SyncOptions {
	map_package_json: GroConfig['map_package_json'];
}

/**
 * What `run_sync` reaches, picked from a builtin's `host` and context.
 */
export interface SyncDeps {
	fs_paths: FsPaths;
	programs: BuiltinPrograms<'npm_cli' | 'sveltekit_cli'>;
	stderr: ProcessOutput;
	invoke_task: BuiltinInvokeTask<BuiltinInvokes<typeof grants>>;
	log: Logger;
}

/**
 * Installs packages, syncs SvelteKit and `package.json`, and runs gen, each optional,
 * `gro sync`'s body.
 * @param args - the task's parsed args
 * @param options - the config's `map_package_json`
 * @param deps - the project's boundary, the programs, stderr, the nested `gen`, and the log
 */
export const run_sync = async (args: Args, options: SyncOptions, deps: SyncDeps): Promise<void> => {
	const { sveltekit, package_json, gen, install } = args;
	const { map_package_json } = options;
	const { fs_paths, programs, stderr, invoke_task, log } = deps;

	if (install) {
		await install_with_cache_healing_or_throw(
			{ program: () => programs.program('npm_cli') },
			{ log, stderr, throw_if_gro_packages_changed: true, fs_paths }
		);
	}

	// a project declaring SvelteKit that isn't installed still gets the install hint
	if (sveltekit && (await project_has_sveltekit_app(fs_paths, log))) {
		await sveltekit_sync({ find: () => programs.find('sveltekit_cli') });
		log.info('synced SvelteKit');
	}

	if (package_json && map_package_json) {
		await package_json_sync(fs_paths, map_package_json, log);
	}

	if (gen) {
		await invoke_task('gen');
	}
};
