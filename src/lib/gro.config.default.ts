import type {CreateGroConfig} from './gro_config.ts';
import {gro_plugin_sveltekit_library} from './gro_plugin_sveltekit_library.ts';
import {has_server, gro_plugin_server} from './gro_plugin_server.ts';
import {gro_plugin_sveltekit_app} from './gro_plugin_sveltekit_app.ts';
import {has_sveltekit_app, has_sveltekit_library} from './sveltekit_helpers.ts';
import {gro_plugin_gen} from './gro_plugin_gen.ts';
import {package_json_load} from './package_json.ts';

// TODO hacky, maybe extract utils?

/**
 * This is the default config that's passed to `gro.config.ts`
 * if it exists in the current project, and if not, this is the final config.
 * It looks at the SvelteKit config and filesystem and tries to do the right thing:
 *
 * - if `svelte.config.js`, assumes a SvelteKit frontend - respects `KitConfig.kit.files.routes`
 * - if `src/lib` + `@sveltejs/package`, assumes a Node library - respects `KitConfig.kit.files.lib`
 * - if `src/lib/server/server.ts`, assumes a Node server - needs config
 */
const config: CreateGroConfig = async (cfg, svelte_config) => {
	const package_json = await package_json_load(); // TODO gets wastefully loaded by some plugins, maybe put in plugin/task context? how does that interact with `map_package_json`?

	const [has_server_result, has_sveltekit_library_result, has_sveltekit_app_result] =
		await Promise.all([
			has_server(),
			has_sveltekit_library(package_json, svelte_config),
			has_sveltekit_app(),
		]);

	// put things that generate files before SvelteKit so it can see them
	cfg.plugins = () =>
		[
			gro_plugin_gen(),
			has_server_result.ok ? gro_plugin_server() : null,
			has_sveltekit_library_result.ok ? gro_plugin_sveltekit_library() : null,
			has_sveltekit_app_result.ok ? gro_plugin_sveltekit_app() : null,
		].filter((v) => v !== null);

	return cfg;
};

export default config;
