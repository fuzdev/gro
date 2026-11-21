import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';

import type { Task } from './task.ts';
import { Plugins, to_plugin_context, type PluginHost } from './plugin.ts';
import { fs } from './fs.ts';
import { create_fs_paths } from './fs_paths.ts';
import { cwd } from './cwd.ts';
import { env } from './env.ts';
import { spawn } from './spawn.ts';
import { import_module_node } from './internal/node_import_module.ts';

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

		// a builtin with no grants, so a root: its plugins get the handles it opens,
		// and the Node project loader
		const host: PluginHost = {
			fs_paths: create_fs_paths(fs, { cwd: cwd(), home: env.get('HOME') }),
			env,
			spawn,
			import_module: import_module_node
		};
		const plugins = await Plugins.create(to_plugin_context(ctx, host, true, watch));
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
