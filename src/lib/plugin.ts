import type { TaskContext } from './task.ts';
import type { Env } from './env.ts';
import type { FsPaths } from './fs_paths.ts';
import type { ImportModule } from './modules.ts';
import type { Spawn } from './spawn.ts';

/**
 * Gro `Plugin`s enable custom behavior during `gro dev` and `gro build`.
 * In contrast, `Adapter`s use the results of `gro build` to produce final artifacts.
 */
export interface Plugin<TPluginContext extends PluginContext = PluginContext> {
	name: string;
	setup?: (ctx: TPluginContext) => void | Promise<void>;
	adapt?: (ctx: TPluginContext) => void | Promise<void>;
	teardown?: (ctx: TPluginContext) => void | Promise<void>;
}

export type PluginsCreateConfig<TPluginContext extends PluginContext = PluginContext> = (
	ctx: TPluginContext
) => Array<Plugin<TPluginContext>> | Promise<Array<Plugin<TPluginContext>>>;

/**
 * The handles a plugin reaches, opened by the task that sets the plugins up, `gro dev` or
 * `gro build`, and handed over at setup, so a plugin module needn't import a host module or `node:`:
 * Gro's plugins and its default config import none when they load, so loading the config,
 * which every command does, reaches none unless the project's own config imports one.
 * The handles are the task's own, uncut: a plugin holds what the task holds.
 */
export interface PluginHost {
	/**
	 * The task's boundary, built from the fs host module with the working directory and `HOME`.
	 */
	readonly fs_paths: FsPaths;
	readonly env: Env;
	readonly spawn: Spawn;
	/**
	 * Loads the project's modules, like the genfiles `gro_plugin_gen` checks.
	 */
	readonly import_module: ImportModule;
	/**
	 * The absolute path of the Node loader of the Gro the task runs a file with,
	 * which a plugin running a file as `gro run` does registers with `spawn_with_loader_args`.
	 */
	readonly loader_path: string;
}

export interface PluginContext<TArgs = object> extends TaskContext<TArgs> {
	dev: boolean;
	watch: boolean;
	host: PluginHost;
}

/**
 * Widens a `TaskContext` to a `PluginContext` by adding `host`, `dev`, and `watch`.
 * Copies property descriptors rather than spreading, because `svelte_config` is a lazy getter -
 * spreading would call it, resolving the Svelte config even for a plugin set that never reads it,
 * and pinning whichever promise it returned instead of the memoized one.
 */
export const to_plugin_context = <TArgs>(
	ctx: TaskContext<TArgs>,
	host: PluginHost,
	dev: boolean,
	watch: boolean
): PluginContext<TArgs> =>
	Object.defineProperties(
		{ dev, watch, host },
		Object.getOwnPropertyDescriptors(ctx)
	) as PluginContext<TArgs>;

/** See `Plugins.create` for a usage example. */
export class Plugins<TPluginContext extends PluginContext> {
	readonly ctx: TPluginContext;
	readonly instances: Array<Plugin<TPluginContext>>;

	constructor(ctx: TPluginContext, instances: Array<Plugin>) {
		this.ctx = ctx;
		this.instances = instances;
	}

	static async create<TPluginContext extends PluginContext>(
		ctx: TPluginContext
	): Promise<Plugins<TPluginContext>> {
		const { timings } = ctx;
		const timing_to_create = timings.start('plugins.create');
		const instances: Array<Plugin> = await ctx.config.plugins(ctx);
		const plugins = new Plugins(ctx, instances);
		timing_to_create();
		return plugins;
	}

	async setup(): Promise<void> {
		const { ctx, instances } = this;
		if (!instances.length) return;
		const { timings, log } = ctx;
		const timing_to_setup = timings.start('plugins.setup');
		for (const plugin of instances) {
			if (!plugin.setup) continue;
			log.debug('setup plugin', plugin.name);
			const timing = timings.start(`setup:${plugin.name}`);
			await plugin.setup(ctx);
			timing();
		}
		timing_to_setup();
	}

	async adapt(): Promise<void> {
		const { ctx, instances } = this;
		const { timings } = ctx;
		const timing_to_run_adapters = timings.start('plugins.adapt');
		for (const plugin of instances) {
			if (!plugin.adapt) continue;
			const timing = timings.start(`adapt:${plugin.name}`);
			await plugin.adapt(ctx);
			timing();
		}
		timing_to_run_adapters();
	}

	async teardown(): Promise<void> {
		const { ctx, instances } = this;
		if (!instances.length) return;
		const { timings, log } = ctx;
		const timing_to_teardown = timings.start('plugins.teardown');
		for (const plugin of instances) {
			if (!plugin.teardown) continue;
			log.debug('teardown plugin', plugin.name);
			const timing = timings.start(`teardown:${plugin.name}`);
			await plugin.teardown(ctx);
			timing();
		}
		timing_to_teardown();
	}
}
