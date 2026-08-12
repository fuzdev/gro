import type * as esbuild from 'esbuild';
import { escape_regexp } from '@fuzdev/fuz_util/regexp.ts';
import { join } from 'node:path';

import { LIB_PATH, SVELTEKIT_LIB_ALIAS } from './constants.ts';

export interface EsbuildPluginSveltekitShimAliasOptions {
	dir?: string;
	alias?: Record<string, string>;
}

export const esbuild_plugin_sveltekit_shim_alias = ({
	dir = process.cwd(),
	alias
}: EsbuildPluginSveltekitShimAliasOptions): esbuild.Plugin => ({
	name: 'sveltekit_shim_alias',
	setup: (build) => {
		// The `$lib` fallback is for callers that pass no `alias` at all -
		// a `ParsedSvelteConfig` always carries one, pointed at its `files.lib`.
		const aliases: Record<string, string> = { [SVELTEKIT_LIB_ALIAS]: LIB_PATH, ...alias };
		// Create a Go-compatible regexp
		const filter = new RegExp(`^(?:${Object.keys(aliases).map(escape_regexp).join('|')})`);
		build.onResolve({ filter }, async (args) => {
			const { path, ...rest } = args;
			// Find which alias prefix matches
			const prefix = Object.keys(aliases).find((key) => path.startsWith(key));
			if (!prefix) return null;
			return build.resolve(join(dir, aliases[prefix] + path.substring(prefix.length)), rest);
		});
	}
});
