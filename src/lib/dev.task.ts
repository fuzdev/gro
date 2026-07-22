import { z } from 'zod';

import type { Task } from './task.ts';
import { Plugins, type PluginContext } from './plugin.ts';
import { clean_fs } from './clean_fs.ts';

/** @nodocs */
export const Args = z.strictObject({
	watch: z.boolean().meta({ description: 'dual of no-watch' }).default(true),
	'no-watch': z
		.boolean()
		.meta({
			description: 'opt out of running a long-lived process to watch files and rebuild on changes'
		})
		.default(false),
	sync: z.boolean().meta({ description: 'dual of no-sync' }).default(true),
	'no-sync': z.boolean().meta({ description: 'opt out of gro sync' }).default(false),
	install: z.boolean().meta({ description: 'opt into installing packages' }).default(false)
});
export type Args = z.infer<typeof Args>;

export type DevTaskContext = PluginContext<Args>;

/** @nodocs */
export const task: Task<Args> = {
	summary: 'start SvelteKit and other dev plugins',
	Args,
	run: async (ctx) => {
		const { args, invoke_task, log } = ctx;
		const { watch, sync, install } = args;

		await clean_fs({ build_dev: true });

		if (sync || install) {
			if (!sync) log.warn('sync is false but install is true, so ignoring the sync option');
			await invoke_task('sync', { install, gen: !watch });
		}

		const plugins = await Plugins.create({ ...ctx, dev: true, watch });
		await plugins.setup();
		if (!watch) {
			await plugins.teardown();
		} else {
			// TODO maybe redesign for this API to be explicitly cancelable?
			// Keep the task running indefinitely in watch mode.
			// This prevents invoke_task from calling finish() and closing the filer.
			await new Promise(() => {
				// Never resolves - keeps filer and listeners alive.
			});
		}
	}
};
