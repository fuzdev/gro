import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';

import type { Task } from './task.ts';
import { package_json_sync } from './package_json.ts';
import { sveltekit_sync } from './sveltekit_helpers.ts';
import { install_with_cache_healing_or_throw } from './npm_install_helpers.ts';

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
	summary: 'run `gro gen`, update `package.json`, and optionally install packages to sync up',
	Args,
	run: async ({ args, invoke_task, config, log }): Promise<void> => {
		const { sveltekit, package_json, gen, install } = args;

		if (install) {
			await install_with_cache_healing_or_throw(config.pm_cli, { log });
		}

		if (sveltekit) {
			await sveltekit_sync(undefined, config.pm_cli);
			log.info('synced SvelteKit');
		}

		if (package_json && config.map_package_json) {
			await package_json_sync(config.map_package_json, log);
		}

		if (gen) {
			await invoke_task('gen');
		}
	}
};
