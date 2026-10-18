# SvelteKit app plugin

Gro's [SvelteKit app plugin](/src/lib/gro_plugin_sveltekit_app.ts)
runs `vite dev` and `vite build` for SvelteKit projects.

> Where it's going: on the [runtime](runtime.md), Vite runs as a subprocess under the run's kernel rules
> until Gro bundles dev and build on its own host and Vite leaves the loop;
> Svelte, Vite, and SvelteKit stay the project's own dependencies throughout.

```ts
// gro.config.ts
import type {CreateGroConfig} from '@fuzdev/gro';
import {gro_plugin_sveltekit_app} from '@fuzdev/gro/gro_plugin_sveltekit_app.ts';

const config: CreateGroConfig = async (cfg) => {
	cfg.plugins = async () => [
		// included in the default config when `@sveltejs/kit` is a dep or dev dep
		// (a peer dep doesn't count - it declares what a package works with, not what it is)
		gro_plugin_sveltekit_app(),
	];
	return cfg;
};

export default config;
```

In development (`gro dev`), spawns `vite dev` with watch mode.

In production (`gro build`), runs `vite build`.
