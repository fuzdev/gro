import {compile, compileModule, preprocess} from 'svelte/compiler';
import {fileURLToPath, pathToFileURL} from 'node:url';
import {dirname, join} from 'node:path';
import type {LoadHook, ResolveHook} from 'node:module';
import {readFileSync} from 'node:fs';
import ts_blank_space from 'ts-blank-space';

import {render_env_shim_module} from './sveltekit_shim_env.ts';
import {
	render_sveltekit_shim_app_environment,
	render_sveltekit_shim_app_paths,
	SVELTEKIT_SHIM_APP_ENVIRONMENT_MATCHER,
	SVELTEKIT_SHIM_APP_PATHS_MATCHER,
	sveltekit_shim_app_specifiers,
} from './sveltekit_shim_app.ts';
import {default_svelte_config} from './svelte_config.ts';
import {paths} from './paths.ts';
import {TS_MATCHER, SVELTE_MATCHER, SVELTE_RUNES_MATCHER} from './constants.ts';
import {resolve_specifier} from './resolve_specifier.ts';
import {map_sveltekit_aliases} from './sveltekit_helpers.ts';

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
node --import 'data:text/javascript,import {register} from "node:module"; import {pathToFileURL} from "node:url"; register("@fuzdev/gro/loader.js", pathToFileURL("./"));' --experimental-import-meta-resolve --experimental-strip-types' foo.ts
```

TODO how to improve that gnarly import line? was originally designed for the now-deprecated `--loader`

*/

// TODO sourcemaps for the svelte preprocessors
// TODO `import.meta.resolve` wasn't available in loaders when this was first implemented, but might be now

// dev is always true in the loader
const dev = true;

const dir = paths.root;

const {
	alias,
	base_url,
	assets_url,
	env_dir,
	private_prefix,
	public_prefix,
	svelte_compile_options,
	svelte_compile_module_options,
	svelte_preprocessors,
} = default_svelte_config;

const aliases = Object.entries(alias);

const RAW_MATCHER = /(%3Fraw|\.css|\.svg)$/; // TODO others? configurable?

/** @nodocs */
export const load: LoadHook = async (url, context, nextLoad) => {
	// console.log(`url`, url);
	if (SVELTEKIT_SHIM_APP_PATHS_MATCHER.test(url)) {
		// SvelteKit `$app/paths` shim
		return {
			format: 'module',
			shortCircuit: true,
			source: render_sveltekit_shim_app_paths(base_url, assets_url),
		};
	} else if (SVELTEKIT_SHIM_APP_ENVIRONMENT_MATCHER.test(url)) {
		// SvelteKit `$app/environment` shim
		return {
			format: 'module',
			shortCircuit: true,
			source: render_sveltekit_shim_app_environment(dev),
		};
	} else if (SVELTE_RUNES_MATCHER.test(url)) {
		// Svelte runes in js/ts, `.svelte.ts`
		const filename = fileURLToPath(url);
		const loaded = await nextLoad(url, {...context, format: 'module-typescript'});
		const raw_source = loaded.source?.toString(); // eslint-disable-line @typescript-eslint/no-base-to-string
		if (raw_source == null) throw Error(`Failed to load ${url}`);
		// TODO should be nice if we could use Node's builtin amaro transform, but I couldn't find a way after digging into the source, AFAICT it's internal and not exposed
		const source = ts_blank_space(raw_source); // TODO was using oxc-transform and probably should, but this doesn't require sourcemaps, and it's still alpha as of May 2025
		const transformed = compileModule(source, {
			...svelte_compile_module_options,
			dev,
			filename,
		});
		return {format: 'module', shortCircuit: true, source: transformed.js.code};
	} else if (TS_MATCHER.test(url)) {
		// ts but not `.svelte.ts`
		return nextLoad(url, {...context, format: 'module-typescript'});
	} else if (SVELTE_MATCHER.test(url)) {
		// Svelte, `.svelte`
		const loaded = await nextLoad(url, {...context, format: 'module'});
		const raw_source = loaded.source!.toString(); // eslint-disable-line @typescript-eslint/no-base-to-string
		const filename = fileURLToPath(url);
		const preprocessed = svelte_preprocessors // TODO @many use sourcemaps (and diagnostics?)
			? await preprocess(raw_source, svelte_preprocessors, {filename})
			: null;
		const source = preprocessed?.code ?? raw_source;
		const transformed = compile(source, {...svelte_compile_options, dev, filename});
		return {format: 'module', shortCircuit: true, source: transformed.js.code};
	} else if (context.importAttributes.type === 'json') {
		// json - any file extension
		// TODO probably follow esbuild and also export every top-level property for objects from the module for good treeshaking - https://esbuild.github.io/content-types/#json (type generation?)
		// TODO why is removing the importAttributes needed? can't pass no context either -
		//   error: `Module "file:///home/user/dev/repo/foo.json" is not of type "json"`
		const loaded = await nextLoad(url, {...context, importAttributes: undefined});
		const raw_source = loaded.source?.toString(); // eslint-disable-line @typescript-eslint/no-base-to-string
		if (raw_source == null) throw Error(`Failed to load ${url}`);
		const source = `export default ` + raw_source;
		return {format: 'module', shortCircuit: true, source};
	} else if (RAW_MATCHER.test(url)) {
		// raw text imports like `?raw`, `.css`, `.svg`
		const filename = fileURLToPath(url.endsWith('%3Fraw') ? url.substring(0, url.length - 6) : url);
		const raw_source = readFileSync(filename, 'utf8');
		const source =
			'export default `' + raw_source.replaceAll('\\', '\\\\').replaceAll('`', '\\`') + '`;';
		return {format: 'module', shortCircuit: true, source};
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
			const source = render_env_shim_module(
				dev,
				mode,
				visibility,
				public_prefix,
				private_prefix,
				env_dir,
			);
			return {format: 'module', shortCircuit: true, source};
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
			importAttributes: {virtual: s}, // TODO idk I'm just making this up
			shortCircuit: true,
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
		return {url, shortCircuit: true};
	}

	return {
		url,
		format: 'module',
		shortCircuit: true,
	};
};
