# plugin

During the [`gro dev`](dev.md) and [`gro build`](build.md) tasks,
Gro uses `Plugin`s to support custom usecases outside of the normal build pipeline.

In this early implementation of plugins in Gro,
plugins run serially, in the order they are returned from `plugins` in the `gro.config.ts`.
Each step of Gro's build processes - `gro dev` for development and `gro build` for production -
runs a method of each plugin, batched together as `setup -> adapt -> teardown`,
with some behavioral inconsistencies:

- `adapt` only runs during production aka `gro build`
- `teardown` does not run for `gro dev` in the default `watch` mode,
  but it does run with `gro dev --no-watch`
- there should probably be a finalization step that runs `teardown` on uncaught exceptions

The API needs to be improved for more advanced usecases,
currently it offers little flexibility -
we'll follow the Vite/SvelteKit APIs probably. (`pre` etc)
Maybe let you map the array of each method batch. (is that possible with those?)

Gro's builtin plugins:

- [`@fuzdev/gro/gro_plugin_server.js`](../lib/gro_plugin_server.ts) - Node server support
- [`@fuzdev/gro/gro_plugin_sveltekit_library.js`](../lib/gro_plugin_sveltekit_library.ts) -
  see [the docs](./gro_plugin_sveltekit_library.md)
- [`@fuzdev/gro/gro_plugin_sveltekit_app.js`](../lib/gro_plugin_sveltekit_app.ts) -
  see [the docs](./gro_plugin_sveltekit_app.md)
- [`@fuzdev/gro/gro_plugin_gen.js`](../lib/gro_plugin_gen.ts) - watch `src/`
  and efficiently run `gen` when genfiles or their deps change

> server and gen plugin docs are not yet written

Also see [`config.plugins` in the config docs](config.md#plugins)
and usage in [the default config](../lib/gro.config.default.ts).
The default config detects which plugins are included by inspecting the current project.

The implementation is at [`src/lib/plugin.ts`](../lib/plugin.ts) with more details.

```ts
export interface Plugin<TPluginContext extends PluginContext = PluginContext> {
	name: string;
	setup?: (ctx: TPluginContext) => void | Promise<void>;
	adapt?: (ctx: TPluginContext) => void | Promise<void>;
	teardown?: (ctx: TPluginContext) => void | Promise<void>;
}

export interface PluginContext<TArgs = object> extends TaskContext<TArgs> {
	dev: boolean;
	watch: boolean;
}
```

The `adapt` step only runs for production during `gro build`, taking after SvelteKit adapters.

## error handling

Plugin errors fail the build immediately, stopping the lifecycle:

- `setup()` error → `adapt` and `teardown` skipped
- `adapt()` error → `teardown` skipped
- `teardown()` error → build fails

This surfaces errors quickly. A future improvement may add cleanup-on-error handling.
