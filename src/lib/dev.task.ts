import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';

import type { Task } from './task.ts';
import { Plugins, to_plugin_context } from './plugin.ts';

/** @nodocs */
export const Args = s.object({
	watch: s.with_default(meta(s.boolean, { description: 'dual of no-watch' }), true),
	'no-watch': s.with_default(
		meta(s.boolean, {
			description: 'opt out of running a long-lived process to watch files and rebuild on changes'
		}),
		false
	),
	sync: s.with_default(meta(s.boolean, { description: 'dual of no-sync' }), true),
	'no-sync': s.with_default(meta(s.boolean, { description: 'opt out of gro sync' }), false),
	install: s.with_default(meta(s.boolean, { description: 'opt into installing packages' }), false)
});
export type Args = s.Infer<typeof Args>;

/** @nodocs */
export const task: Task<Args> = {
	summary: 'start SvelteKit and other dev plugins',
	Args,
	run: async (ctx) => {
		const { task_args, invoke_task, log } = ctx;
		const { watch, sync, install } = task_args;

		if (sync || install) {
			if (!sync) log.warn('sync is false but install is true, so ignoring the sync option');
			// `gro_plugin_gen` owns gen in dev, so sync's would run it twice
			await invoke_task('sync', { install, gen: false });
		}

		const plugins = await Plugins.create(to_plugin_context(ctx, true, watch));
		await plugins.setup();
		if (!watch) {
			await plugins.teardown();
		} else {
			// TODO maybe redesign for this API to be explicitly cancelable?
			// Keep the task running indefinitely in watch mode.
			// This keeps invoke_task from closing the graph it created.
			await new Promise(() => {
				// Never resolves - keeps the graph and listeners alive.
			});
		}
	}
};
