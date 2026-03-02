# SvelteKit library plugin

Gro's [SvelteKit library plugin](/src/lib/gro_plugin_sveltekit_library.ts)
uses [`svelte-package`](https://svelte.dev/docs/kit/packaging)
to build libraries from `src/lib/` for publishing to npm.

## detection

The [default config](/src/lib/gro.config.default.ts) enables this plugin
when all three conditions are met:

1. `svelte.config.js` exists at the project root
2. `src/lib/` directory exists (or the path configured in `svelte.config.js`)
3. `@sveltejs/package` is listed in `package.json` dependencies

Install to enable:

```bash
npm i -D @sveltejs/package
```

## behavior

In production (`gro build`), runs `svelte-package` during `setup`
to compile `src/lib/` into `dist/`.

In development (`gro dev`), does nothing — `svelte-package` is a build-time tool.

During `adapt`, if `package.json` has a `bin` field,
the plugin makes the binaries executable and runs `npm link -f`
so CLI commands are available locally after building.

## configuration

```ts
// gro.config.ts
import type {CreateGroConfig} from '@fuzdev/gro';
import {gro_plugin_sveltekit_library} from '@fuzdev/gro/gro_plugin_sveltekit_library.js';

const config: CreateGroConfig = async (cfg) => {
	cfg.plugins = async () => [
		// included in the default config when detection passes
		gro_plugin_sveltekit_library({
			// svelte_package_options: {output: 'custom_dist'},
			// svelte_package_cli: 'svelte-package',
		}),
	];
	return cfg;
};

export default config;
```

Options are forwarded to `svelte-package`.
See [`SveltePackageOptions`](/src/lib/sveltekit_helpers.ts)
and the [SvelteKit packaging docs](https://svelte.dev/docs/kit/packaging#options).

## exports

When this plugin is active, `gro sync` auto-generates `package.json` `"exports"`
using wildcard subpath patterns for `.js`, `.ts`, `.svelte`, `.json`, and `.css` files in `src/lib/`.
Customize via [`map_package_json` in the config](config.md#map_package_json).

For the full publishing workflow, see [publish.md](publish.md).
