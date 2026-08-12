import type { CreateGroConfig } from './gro_config.ts';
import { gro_plugin_sveltekit_library } from './gro_plugin_sveltekit_library.ts';
import { has_server, gro_plugin_server } from './gro_plugin_server.ts';
import { gro_plugin_sveltekit_app } from './gro_plugin_sveltekit_app.ts';
import { has_sveltekit_app, has_sveltekit_library } from './sveltekit_helpers.ts';
import { gro_plugin_gen } from './gro_plugin_gen.ts';
import { package_json_load } from './package_json.ts';

// TODO hacky, maybe extract utils?

/**
 * This is the default config that's passed to `gro.config.ts`
 * if it exists in the current project, and if not, this is the final config.
 * It looks at `package.json` and the filesystem and tries to do the right thing:
 *
 * - if `@sveltejs/kit`, assumes a SvelteKit frontend
 * - if `@sveltejs/package` + the lib directory, assumes a Node library - respects `KitConfig.kit.files.lib`
 * - if `src/lib/server/server.ts`, assumes a Node server - needs config
 */
const config: CreateGroConfig = (cfg) => {
	// Detection is deferred into `plugins` because every Gro invocation loads the config,
	// but only `dev` and `build` create plugins - this keeps `package.json`
	// and the SvelteKit config off the path of every other task.
	cfg.plugins = async () => {
		const package_json = await package_json_load(); // TODO gets wastefully loaded by some plugins, maybe put in plugin/task context? how does that interact with `map_package_json`?

		// `has_server` reads the Svelte config, because the server's location follows
		// `kit.files.lib` and there's no way to find it without knowing where that points.
		// So `dev` and `build` resolve the config once here no matter what the project is -
		// which is the right place to pay for it, since they're the commands that need it.
		const has_sveltekit_app_result = has_sveltekit_app(package_json);
		const [has_server_result, has_sveltekit_library_result] = await Promise.all([
			has_server(),
			has_sveltekit_library(package_json)
		]);

		// put things that generate files before SvelteKit so it can see them
		return [
			gro_plugin_gen(),
			has_server_result.ok ? gro_plugin_server() : null,
			has_sveltekit_library_result.ok ? gro_plugin_sveltekit_library() : null,
			has_sveltekit_app_result.ok ? gro_plugin_sveltekit_app() : null
		].filter((v) => v !== null);
	};

	return cfg;
};

export default config;
