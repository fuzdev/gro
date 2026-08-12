import { compile, compileModule, preprocess } from 'svelte/compiler';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { dirname, join } from 'node:path';
import type { LoadHook, ResolveHook } from 'node:module';
import { readFileSync } from 'node:fs';
import ts_blank_space from 'ts-blank-space';

import { render_env_shim_module } from './sveltekit_shim_env.ts';
import {
	render_sveltekit_shim_app_environment,
	render_sveltekit_shim_app_paths,
	SVELTEKIT_SHIM_APP_ENVIRONMENT_MATCHER,
	SVELTEKIT_SHIM_APP_PATHS_MATCHER,
	sveltekit_shim_app_specifiers
} from './sveltekit_shim_app.ts';
import {
	has_vite_config,
	load_default_svelte_config,
	warn_svelte_config_ignored,
	NO_SVELTE_PLUGIN_REASON
} from './svelte_config.ts';
import {
	svelte_config_cache_read,
	svelte_config_cache_stamps,
	svelte_config_cache_write
} from './svelte_config_cache.ts';
import { paths } from './paths.ts';
import { TS_MATCHER, SVELTE_MATCHER, SVELTE_RUNES_MATCHER } from './constants.ts';
import { resolve_specifier } from './resolve_specifier.ts';
import { map_sveltekit_aliases } from './sveltekit_helpers.ts';

// TODO get out of the loader business, starting with https://nodejs.org/api/typescript.html#type-stripping

/*

Usage via `$lib/register.ts`:

```bash
node --import @fuzdev/gro/register.js foo.ts
```

Usage via `$lib/run.task.ts`:

```bash
gro run foo.ts
```

Direct usage without register (see also `$lib/gro.ts`):

```bash
node --import 'data:text/javascript,import {register} from "node:module"; import {pathToFileURL} from "node:url"; register("@fuzdev/gro/loader.ts", pathToFileURL("./"));' --experimental-import-meta-resolve --experimental-strip-types' foo.ts
```

TODO how to improve that gnarly import line? was originally designed for the now-deprecated `--loader`

*/

// TODO sourcemaps for the svelte preprocessors
// TODO `import.meta.resolve` wasn't available in loaders when this was first implemented, but might be now

// dev is always true in the loader
const dev = true;

const dir = paths.root;

/*

`resolve` is the one hook that can't await the config.

Resolving it imports the Vite config, and the hooks thread's own imports go back through
its own hooks - so a `resolve` that awaited the load it is part of re-enters itself until
the stack blows. `load` has no such problem, and it's where all but one of the config's
fields are read. So the alias map is the only thing needed up front, and it's the only
thing cached: see `svelte_config_cache.ts`.

On a hit, nothing is resolved here and the rest of the config is awaited inside `load`,
which most invocations never reach - tasks and genfiles are TypeScript, and `gro test`
hands its files to Vitest rather than to this loader. On a miss the whole config loads
here at module scope, before the hooks go live, which keeps that import graph out of them.

The one shape this can't survive is a Vite config that imports a module `load` resolves
the config for - a `.svelte`, `.svelte.ts`, `$env`, or `$app/paths` import reached from
the config graph would await a load it is part of. Nothing puts those in a Vite config,
and there's no correct value to hand back if something did.

The loader runs on a worker thread, where `process.chdir` is unavailable - Vite's own
config resolution doesn't need it, so this is the same load the main thread does.
Both read the project in the cwd, which is the only project either one resolves.

*/
const cache_stamps = svelte_config_cache_stamps();
const cached_svelte_config = svelte_config_cache_read(cache_stamps);

let aliases: Array<[string, string]>;
if (cached_svelte_config) {
	aliases = Object.entries(cached_svelte_config.alias);
	if (!cached_svelte_config.svelte_config_found) {
		warn_svelte_config_ignored(process.cwd(), NO_SVELTE_PLUGIN_REASON);
	}
} else {
	const parsed_svelte_config = await load_default_svelte_config();
	aliases = Object.entries(parsed_svelte_config.alias);
	// Skipped without a Vite config because that path resolves nothing to save,
	// and because its warning has to repeat rather than be cached away.
	if (has_vite_config()) {
		svelte_config_cache_write(cache_stamps, {
			alias: parsed_svelte_config.alias,
			svelte_config_found: parsed_svelte_config.svelte_config !== null
		});
	}
}

const RAW_MATCHER = /(%3Fraw|\.css|\.svg)$/; // TODO others? configurable?

/** @nodocs */
export const load: LoadHook = async (url, context, nextLoad) => {
	// console.log(`url`, url);
	if (SVELTEKIT_SHIM_APP_PATHS_MATCHER.test(url)) {
		// SvelteKit `$app/paths` shim
		const { base_url, assets_url } = await load_default_svelte_config();
		return {
			format: 'module',
			shortCircuit: true,
			source: render_sveltekit_shim_app_paths(base_url, assets_url)
		};
	} else if (SVELTEKIT_SHIM_APP_ENVIRONMENT_MATCHER.test(url)) {
		// SvelteKit `$app/environment` shim
		return {
			format: 'module',
			shortCircuit: true,
			source: render_sveltekit_shim_app_environment(dev)
		};
	} else if (SVELTE_RUNES_MATCHER.test(url)) {
		// Svelte runes in js/ts, `.svelte.ts`
		const filename = fileURLToPath(url);
		const loaded = await nextLoad(url, { ...context, format: 'module-typescript' });
		const raw_source = loaded.source?.toString(); // eslint-disable-line @typescript-eslint/no-base-to-string
		if (raw_source == null) throw Error(`Failed to load ${url}`);
		// TODO should be nice if we could use Node's builtin amaro transform, but I couldn't find a way after digging into the source, AFAICT it's internal and not exposed
		const source = ts_blank_space(raw_source); // TODO was using oxc-transform and probably should, but this doesn't require sourcemaps, and it's still alpha as of May 2025
		const { svelte_compile_module_options } = await load_default_svelte_config();
		const transformed = compileModule(source, {
			...svelte_compile_module_options,
			dev,
			filename
		});
		return { format: 'module', shortCircuit: true, source: transformed.js.code };
	} else if (TS_MATCHER.test(url)) {
		// ts but not `.svelte.ts`
		return nextLoad(url, { ...context, format: 'module-typescript' });
	} else if (SVELTE_MATCHER.test(url)) {
		// Svelte, `.svelte`
		const loaded = await nextLoad(url, { ...context, format: 'module' });
		const raw_source = loaded.source!.toString(); // eslint-disable-line @typescript-eslint/no-base-to-string
		const filename = fileURLToPath(url);
		const { svelte_compile_options, svelte_preprocessors } = await load_default_svelte_config();
		const preprocessed = svelte_preprocessors // TODO @many use sourcemaps (and diagnostics?)
			? await preprocess(raw_source, svelte_preprocessors, { filename })
			: null;
		const source = preprocessed?.code ?? raw_source;
		const transformed = compile(source, { ...svelte_compile_options, dev, filename });
		return { format: 'module', shortCircuit: true, source: transformed.js.code };
	} else if (context.importAttributes.type === 'json') {
		// json - any file extension
		// TODO probably follow esbuild and also export every top-level property for objects from the module for good treeshaking - https://esbuild.github.io/content-types/#json (type generation?)
		// TODO why is removing the importAttributes needed? can't pass no context either -
		//   error: `Module "file:///home/user/dev/repo/foo.json" is not of type "json"`
		const loaded = await nextLoad(url, { ...context, importAttributes: undefined });
		const raw_source = loaded.source?.toString(); // eslint-disable-line @typescript-eslint/no-base-to-string
		if (raw_source == null) throw Error(`Failed to load ${url}`);
		const source = `export default ` + raw_source;
		return { format: 'module', shortCircuit: true, source };
	} else if (RAW_MATCHER.test(url)) {
		// raw text imports like `?raw`, `.css`, `.svg`
		const filename = fileURLToPath(url.endsWith('%3Fraw') ? url.substring(0, url.length - 6) : url);
		const raw_source = readFileSync(filename, 'utf8');
		const source =
			'export default `' + raw_source.replaceAll('\\', '\\\\').replaceAll('`', '\\`') + '`;';
		return { format: 'module', shortCircuit: true, source };
	} else {
		// SvelteKit `$env`
		// TODO use `format` from the resolve hook to speed this up and make it simpler
		if (context.format === 'sveltekit-env') {
			let mode: 'static' | 'dynamic';
			let visibility: 'public' | 'private';
			switch (context.importAttributes.virtual) {
				case '$env/static/public': {
					mode = 'static';
					visibility = 'public';
					break;
				}
				case '$env/static/private': {
					mode = 'static';
					visibility = 'private';
					break;
				}
				case '$env/dynamic/public': {
					mode = 'dynamic';
					visibility = 'public';
					break;
				}
				case '$env/dynamic/private': {
					mode = 'dynamic';
					visibility = 'private';
					break;
				}
				default: {
					throw Error(`Unknown $env import: ${context.importAttributes.virtual}`);
				}
			}
			const { env_dir, private_prefix, public_prefix } = await load_default_svelte_config();
			const source = render_env_shim_module(
				dev,
				mode,
				visibility,
				public_prefix,
				private_prefix,
				env_dir
			);
			return { format: 'module', shortCircuit: true, source };
		}
	}

	// fallback to default behavior
	return nextLoad(url, context);
};

/** @nodocs */
export const resolve: ResolveHook = async (specifier, context, nextResolve) => {
	let s = specifier;

	// Support SvelteKit `$env` imports
	if (
		s === '$env/static/public' ||
		s === '$env/static/private' ||
		s === '$env/dynamic/public' ||
		s === '$env/dynamic/private'
	) {
		// The returned `url` is validated before `load` is called,
		// so we need a slightly roundabout strategy to pass through the specifier for virtual files.
		return {
			url: pathToFileURL(join(dir, 'src/lib', s)).href,
			format: 'sveltekit-env',
			importAttributes: { virtual: s }, // TODO idk I'm just making this up
			shortCircuit: true
		};
	}

	// Support SvelteKit `$app` imports, including from node_modules
	const shimmed = sveltekit_shim_app_specifiers.get(s);
	if (shimmed !== undefined) {
		return nextResolve(shimmed, context);
	}

	// Apply SvelteKit aliases (handles self-referencing packages like @fuzdev/fuz_util -> src/lib)
	const original = s;
	s = map_sveltekit_aliases(s, aliases);

	// Bare specifiers (not starting with . or /) use Node's default resolution
	if (s[0] !== '.' && s[0] !== '/') {
		return nextResolve(s, context);
	}

	// Resolve paths using Vite conventions
	const parent_url = context.parentURL;
	if (!parent_url) {
		return nextResolve(s, context);
	}

	// Fast path: parent inside `node_modules` and aliasing didn't transform the specifier.
	// Defer to Node's default resolution and format detection — skips the fs.stat work in
	// `resolve_specifier` and preserves CJS/ESM interop. e.g. `ws/wrapper.mjs` statically
	// imports `./lib/permessage-deflate.js` (CJS) and reads its `default` export, which Node
	// only synthesizes when the file is loaded as CommonJS.
	if (s === original && parent_url.includes('/node_modules/')) {
		return nextResolve(s, context);
	}

	const resolved = await resolve_specifier(s, dirname(fileURLToPath(parent_url)));
	const url = pathToFileURL(resolved.path_id_with_querystring).href;

	// Safety net for less common routes into `node_modules` (an alias that maps there, or
	// project code doing `import './node_modules/...'`): same CJS interop reason as above.
	if (url.includes('/node_modules/')) {
		return { url, shortCircuit: true };
	}

	return {
		url,
		format: 'module',
		shortCircuit: true
	};
};
