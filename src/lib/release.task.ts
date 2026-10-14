import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';

import type { Task } from './task.ts';
import { has_sveltekit_library, has_sveltekit_app } from './sveltekit_helpers.ts';
import { package_json_load } from './package_json.ts';

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

/** @nodocs */
export const task: Task<Args> = {
	summary: 'publish and deploy',
	Args,
	run: async ({ args, invoke_task }) => {
		const { dry, check, build, pull, sync, install, gen, force_build } = args;

		const package_json = await package_json_load();

		const publish = (await has_sveltekit_library(package_json)).ok;
		if (publish) {
			await invoke_task('publish', { optional: true, dry, check, build, pull, sync, install });
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
	}
};
