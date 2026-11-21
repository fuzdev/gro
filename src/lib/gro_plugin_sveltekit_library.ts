import type { Plugin } from './plugin.ts';
import { package_json_load } from './package_json.ts';
import { to_forwarded_argv, to_forwarded_flag_value } from './task_args.ts';
import { run_svelte_package, type SveltePackageOptions } from './sveltekit_helpers.ts';
import { SVELTE_PACKAGE_CLI, SVELTEKIT_DIST_DIRNAME } from './constants.ts';
import { rewrite_dist_imports } from './dist_rewrite_imports.ts';
import { is_fs_error } from './fs_error.ts';
import type { FsPaths } from './fs_paths.ts';
import { format_error_detail } from './error_detail.ts';
import { assert_no_forwarded_info_flag } from './internal/forwarded_info_flags.ts';

export interface GroPluginSveltekitLibraryOptions {
	/**
	 * The options passed to the SvelteKit packaging CLI.
	 * @see https://svelte.dev/docs/kit/packaging#options
	 */
	svelte_package_options?: SveltePackageOptions;
}

/**
 * Makes `bin_path` executable for everyone, its mode's other bits kept,
 * returning what went wrong, or `null` when nothing did.
 */
const chmod_bin = async (fs_paths: FsPaths, bin_path: string): Promise<string | null> => {
	try {
		const { dir, path } = await fs_paths.open_path(bin_path);
		const info = await dir.stat(path);
		if (info === null) return `not_found: nothing is at ${bin_path}`;
		// `set_mode` takes only the permission bits, which is all a bin has
		await dir.set_mode(path, (info.mode | 0o111) & 0o777);
		return null;
	} catch (error) {
		if (is_fs_error(error)) return format_error_detail(error);
		throw error;
	}
};

export const gro_plugin_sveltekit_library = ({
	svelte_package_options
}: GroPluginSveltekitLibraryOptions = {}): Plugin => {
	return {
		name: 'gro_plugin_sveltekit_library',
		setup: async ({ dev, log, raw_rest_args, host: { fs_paths, spawn } }) => {
			if (!dev) {
				const package_json = await package_json_load(fs_paths);
				const forwarded_argv = to_forwarded_argv(SVELTE_PACKAGE_CLI, raw_rest_args);
				assert_no_forwarded_info_flag(SVELTE_PACKAGE_CLI, forwarded_argv, true);
				await run_svelte_package(fs_paths, spawn, {
					package_json,
					svelte_package_options,
					forwarded_argv,
					log
				});
				// `svelte-package` ships `.svelte` source verbatim and leaves relative `.ts`
				// specifiers in `.d.ts` declarations; rewrite them to `.js` so the published
				// `dist` resolves for external consumers without `allowImportingTsExtensions`.
				// a forwarded `-- svelte-package --output x` wins, like it does for `svelte-package`
				const output_dir =
					to_forwarded_flag_value(forwarded_argv, 'output', 'o') ??
					svelte_package_options?.output ??
					svelte_package_options?.o ??
					SVELTEKIT_DIST_DIRNAME;
				await rewrite_dist_imports(fs_paths, output_dir, log);
			}
		},
		adapt: async ({ log, host: { fs_paths } }) => {
			const package_json = await package_json_load(fs_paths);
			// a build recreates the binaries, and an `npm link` of them runs them in place
			if (package_json.bin) {
				await Promise.all(
					Object.values(package_json.bin).map(async (bin_path) => {
						const message = await chmod_bin(fs_paths, bin_path);
						if (message !== null) log.error(`chmod on bin path ${bin_path} failed: ${message}`);
					})
				);
			}
		}
	};
};
