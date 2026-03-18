import {z} from 'zod';

import type {Task} from './task.ts';
import {has_sveltekit_library, has_sveltekit_app} from './sveltekit_helpers.ts';
import {package_json_load} from './package_json.ts';

/** @nodocs */
export const Args = z.strictObject({
	dry: z
		.boolean()
		.meta({description: 'build and prepare without actually publishing or deploying'})
		.default(false),
	check: z.boolean().meta({description: 'dual of no-check'}).default(true),
	'no-check': z
		.boolean()
		.meta({description: 'opt out of checking before publishing'})
		.default(false),
	build: z.boolean().meta({description: 'dual of no-build'}).default(true),
	'no-build': z.boolean().meta({description: 'opt out of building'}).default(false),
	pull: z.boolean().meta({description: 'dual of no-pull'}).default(true),
	'no-pull': z.boolean().meta({description: 'opt out of git pull'}).default(false),
	sync: z.boolean().meta({description: 'dual of no-sync'}).default(true),
	'no-sync': z.boolean().meta({description: 'opt out of gro sync'}).default(false),
	install: z.boolean().meta({description: 'dual of no-install'}).default(true),
	'no-install': z
		.boolean()
		.meta({description: 'opt out of installing packages'})
		.default(false),
	gen: z.boolean().meta({description: 'dual of no-gen'}).default(true),
	'no-gen': z.boolean().meta({description: 'opt out of gro gen in deploy build'}).default(false),
	force_build: z
		.boolean()
		.meta({description: 'force a fresh build, ignoring the cache'})
		.default(false),
});
export type Args = z.infer<typeof Args>;

/** @nodocs */
export const task: Task<Args> = {
	summary: 'publish and deploy',
	Args,
	run: async ({args, invoke_task}) => {
		const {dry, check, build, pull, sync, install, gen, force_build} = args;

		const package_json = await package_json_load();

		const publish = (await has_sveltekit_library(package_json)).ok;
		if (publish) {
			await invoke_task('publish', {optional: true, dry, check, build, pull, sync, install});
		}
		if ((await has_sveltekit_app()).ok) {
			await invoke_task('deploy', {
				build: build && !publish,
				dry,
				pull,
				sync,
				install,
				gen,
				force_build,
			});
		}
	},
};
