import { compile, compileModule, preprocess } from 'svelte/compiler';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { dirname, join } from 'node:path';
import type { LoadHook, ResolveHook } from 'node:module';
import { readFileSync } from 'node:fs';
import { AsyncLocalStorage } from 'node:async_hooks';
import ts_blank_space from 'ts-blank-space';

import {
	load_sveltekit_env_declarations,
	render_sveltekit_shim_app_env_vars,
	resolve_sveltekit_env_entry,
	SVELTEKIT_APP_ENV_VARS_MATCHER
} from './sveltekit_shim_app_env_vars.ts';
import {
	render_sveltekit_shim_app_env,
	render_sveltekit_shim_app_paths,
	SVELTEKIT_SHIM_APP_ENV_MATCHER,
	SVELTEKIT_SHIM_APP_PATHS_MATCHER,
	sveltekit_shim_app_specifiers
} from './sveltekit_shim_app.ts';
import { load_default_svelte_config } from './svelte_config.ts';
import { paths } from './paths.ts';
import { TS_MATCHER, SVELTE_MATCHER, SVELTE_RUNES_MATCHER } from './constants.ts';
import { resolve_specifier, to_missing_module_id } from './resolve_specifier.ts';

// TODO get out of the loader business, starting with https://nodejs.org/api/typescript.html#type-stripping

/*

Usage via `register.ts`:

```bash
node --import @fuzdev/gro/register.js foo.ts
```

Usage via `run.task.ts`:

```bash
gro run foo.ts
```

Direct usage without register (see also `gro.ts`):

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

// The shims resolve to the loader's own sibling modules rather than `@fuzdev/gro/*`,
// so they work in projects that run a global Gro without installing it.
// The loader runs from `src/lib` as `.ts` or from `dist` as `.js`.
const sibling_ext = import.meta.url.endsWith('.ts') ? '.ts' : '.js';
const to_sibling_url = (name: string): string => new URL(name + sibling_ext, import.meta.url).href;
const env_module_url = to_sibling_url('env');
/**
 * Set while the loader imports a project's `src/env.ts` for its declarations.
 * The hooks thread's own imports run through these hooks inside that async context,
 * so an `$app/env/*` load seen within it is `src/env.ts` importing the module it declares,
 * which would otherwise wait on itself forever and exit silently.
 */
const loading_env_declarations = new AsyncLocalStorage<string>();

const shim_app_urls = new Map(
	Array.from(sveltekit_shim_app_specifiers, ([specifier, shim]) => [
		specifier,
		to_sibling_url(shim.slice(shim.lastIndexOf('/') + 1, -'.js'.length))
	])
);

/*

`resolve` needs no config - SvelteKit 3's `#lib/*` subpath imports resolve natively,
and the deprecated `alias` option isn't supported. The config is awaited only inside
`load`, which most invocations never reach - tasks and genfiles are TypeScript, and
`gro test` hands its files to Vitest rather than to this loader.

The one shape this can't survive is a Vite config that imports a module `load` resolves
the config for - a `.svelte`, `.svelte.ts`, `$app/env`, or `$app/paths` import reached from
the config graph would await a load it is part of. Nothing puts those in a Vite config,
and there's no correct value to hand back if something did.

The loader runs on a worker thread, where `process.chdir` is unavailable - Vite's own
config resolution doesn't need it, so this is the same load the main thread does.
Both read the project in the cwd, which is the only project either one resolves.

*/

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
	} else if (SVELTEKIT_SHIM_APP_ENV_MATCHER.test(url)) {
		// SvelteKit `$app/env` shim
		const { version_name } = await load_default_svelte_config();
		return {
			format: 'module',
			shortCircuit: true,
			source: render_sveltekit_shim_app_env(dev, version_name)
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
		// SvelteKit `$app/env/public` and `$app/env/private`
		// TODO use `format` from the resolve hook to speed this up and make it simpler
		if (context.format === 'sveltekit-env') {
			const virtual = context.importAttributes.virtual;
			const matches = typeof virtual === 'string' && SVELTEKIT_APP_ENV_VARS_MATCHER.exec(virtual);
			if (!matches) throw Error(`Unknown $app/env import: ${virtual}`);
			const declaring = loading_env_declarations.getStore();
			if (declaring !== undefined) {
				const message = `${declaring} can't import ${virtual} - it declares the env vars that module exports`;
				return {
					format: 'module',
					shortCircuit: true,
					source: `throw Error(${JSON.stringify(message)});`
				};
			}
			const visibility = matches[1] as 'public' | 'private';
			const { src_path, env_dir } = await load_default_svelte_config();
			const entry_id = resolve_sveltekit_env_entry(src_path);
			const variables = entry_id
				? await loading_env_declarations.run(entry_id, () =>
						load_sveltekit_env_declarations(entry_id)
					)
				: null;
			const source = render_sveltekit_shim_app_env_vars({
				visibility,
				variables,
				entry_specifier: entry_id && pathToFileURL(entry_id).href,
				env_module_specifier: env_module_url,
				dev,
				env_dir
			});
			return { format: 'module', shortCircuit: true, source };
		}
	}

	// fallback to default behavior
	return nextLoad(url, context);
};

/** @nodocs */
export const resolve: ResolveHook = async (specifier, context, nextResolve) => {
	const s = specifier;

	// Support SvelteKit `$app/env/public` and `$app/env/private` imports
	if (SVELTEKIT_APP_ENV_VARS_MATCHER.test(s)) {
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
	const shimmed = shim_app_urls.get(s);
	if (shimmed !== undefined) {
		return nextResolve(shimmed, context);
	}

	// Subpath imports like `#lib/*` resolve through package.json `imports`,
	// falling back to the Vite convention of `.js` naming a `.ts` file
	if (s[0] === '#') {
		try {
			return await nextResolve(s, context);
		} catch (error) {
			const id = to_missing_module_id(error);
			if (id === null) throw error;
			const resolved = await resolve_specifier(id, dir);
			return {
				url: pathToFileURL(resolved.path_id_with_querystring).href,
				format: 'module',
				shortCircuit: true
			};
		}
	}

	// Bare specifiers (not starting with . or /) use Node's default resolution
	if (s[0] !== '.' && s[0] !== '/') {
		return nextResolve(s, context);
	}

	// Resolve paths using Vite conventions
	const parent_url = context.parentURL;
	if (!parent_url) {
		return nextResolve(s, context);
	}

	// Fast path: parent inside `node_modules`.
	// Defer to Node's default resolution and format detection — skips the fs.stat work in
	// `resolve_specifier` and preserves CJS/ESM interop. e.g. `ws/wrapper.mjs` statically
	// imports `./lib/permessage-deflate.js` (CJS) and reads its `default` export, which Node
	// only synthesizes when the file is loaded as CommonJS.
	if (parent_url.includes('/node_modules/')) {
		return nextResolve(s, context);
	}

	const resolved = await resolve_specifier(s, dirname(fileURLToPath(parent_url)));
	const url = pathToFileURL(resolved.path_id_with_querystring).href;

	// Safety net for project code doing `import './node_modules/...'`:
	// same CJS interop reason as above.
	if (url.includes('/node_modules/')) {
		return { url, shortCircuit: true };
	}

	return {
		url,
		format: 'module',
		shortCircuit: true
	};
};
