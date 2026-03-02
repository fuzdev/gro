import {print_spawn_result, spawn, spawn_result_to_message} from '@fuzdev/fuz_util/process.js';

import type {Plugin} from './plugin.ts';
import {TaskError} from './task.ts';
import {package_json_load} from './package_json.ts';
import {run_svelte_package, type SveltePackageOptions} from './sveltekit_helpers.ts';
import {SVELTE_PACKAGE_CLI} from './constants.ts';

export interface GroPluginSveltekitLibraryOptions {
	/**
	 * The options passed to the SvelteKit packaging CLI.
	 * @see https://svelte.dev/docs/kit/packaging#options
	 */
	svelte_package_options?: SveltePackageOptions;
	/**
	 * The SvelteKit packaging CLI to use. Defaults to `svelte-package`.
	 * @see https://svelte.dev/docs/kit/packaging
	 */
	svelte_package_cli?: string;
}

export const gro_plugin_sveltekit_library = ({
	svelte_package_options,
	svelte_package_cli = SVELTE_PACKAGE_CLI,
}: GroPluginSveltekitLibraryOptions = {}): Plugin => {
	return {
		name: 'gro_plugin_sveltekit_library',
		setup: async ({dev, log, config}) => {
			if (!dev) {
				const package_json = await package_json_load();
				await run_svelte_package(
					package_json,
					svelte_package_options,
					svelte_package_cli,
					log,
					config.pm_cli,
				);
			}
		},
		adapt: async ({log, timings, config}) => {
			const package_json = await package_json_load();
			// link the CLI binaries if they exist
			if (package_json.bin) {
				const timing_to_link = timings.start(`${config.pm_cli} link`);
				await Promise.all(
					Object.values(package_json.bin).map(async (bin_path) => {
						const chmod_result = await spawn('chmod', ['+x', bin_path]);
						if (!chmod_result.ok)
							log.error(
								`chmod on bin path ${bin_path} failed: ${spawn_result_to_message(chmod_result)}`,
							);
					}),
				);
				log.info(`linking`);
				const link_result = await spawn(config.pm_cli, ['link', '-f']); // TODO don't use `-f` unless necessary or at all?
				if (!link_result.ok) {
					throw new TaskError(`Failed to link. ${print_spawn_result(link_result)}`);
				}
				timing_to_link();
			}
		},
	};
};
