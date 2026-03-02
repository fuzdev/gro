# SvelteKit app plugin

Gro's [SvelteKit app plugin](/src/lib/gro_plugin_sveltekit_app.ts)
runs `vite dev` and `vite build` for SvelteKit projects.

```ts
// gro.config.ts
import type {CreateGroConfig} from '@fuzdev/gro';
import {gro_plugin_sveltekit_app} from '@fuzdev/gro/gro_plugin_sveltekit_app.js';

const config: CreateGroConfig = async (cfg) => {
	cfg.plugins = async () => [
		// included in the default config for SvelteKit projects with src/routes/
		gro_plugin_sveltekit_app(),
	];
	return cfg;
};

export default config;
```

In development (`gro dev`), spawns `vite dev` with watch mode.

In production (`gro build`), runs `vite build`.

For publishing `.well-known/` metadata, see [`vite_plugin_library_well_known`](https://ui.fuz.dev/docs/vite_plugin_library_well_known) (now in `@fuzdev/fuz_ui`).
