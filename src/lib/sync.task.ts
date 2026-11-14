import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';

import type { Task } from './task.ts';
import { package_json_sync } from './package_json.ts';
import { project_has_sveltekit_app, sveltekit_sync } from './sveltekit_helpers.ts';
import { install_with_cache_healing_or_throw } from './npm_install_helpers.ts';
import { process } from './process.ts';
import { fs } from './fs.ts';
import { spawn } from './spawn.ts';
import { create_fs_paths } from './fs_paths.ts';

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

/** @nodocs */
export const task: Task<Args> = {
	summary: 'optionally install packages, sync SvelteKit, update `package.json`, and run `gro gen`',
	Args,
	run: async ({ task_args, invoke_task, config, log }): Promise<void> => {
		const { sveltekit, package_json, gen, install } = task_args;
		const fs_paths = create_fs_paths(fs);

		if (install) {
			await install_with_cache_healing_or_throw(spawn, config.pm_cli, {
				log,
				stderr: process.stderr,
				throw_if_gro_packages_changed: true,
				fs_paths
			});
		}

		// a project declaring SvelteKit that isn't installed still gets the install hint
		if (sveltekit && (await project_has_sveltekit_app(fs_paths, log))) {
			await sveltekit_sync(spawn, { pm_cli: config.pm_cli });
			log.info('synced SvelteKit');
		}

		if (package_json && config.map_package_json) {
			await package_json_sync(fs_paths, config.map_package_json, log);
		}

		if (gen) {
			await invoke_task('gen');
		}
	}
};
