/**
 * Gro build plugin for compiling TypeScript to a standalone binary via `deno compile`.
 *
 * Runs during the `adapt` phase (production build only).
 * Optionally generates a SHA-256 hash file for verification.
 *
 * To avoid bundling all npm devDependencies from `package.json`, we:
 * 1. Extract just the "imports" from root deno.json
 * 2. Rebase relative paths (e.g. `../fuz_app/` → `../../fuz_app/`) since
 *    compilation runs from the output subdirectory
 * 3. Write a minimal deno.json to the output directory
 * 4. Compile from that directory (which has no `package.json`)
 *
 * @module
 */

import {spawn, spawn_result_to_message} from '@fuzdev/fuz_util/process.js';
import {hash_sha256} from '@fuzdev/fuz_util/hash.js';
import type {Plugin} from './plugin.js';
import {TaskError} from './task.js';
import {join, relative} from 'node:path';
import {readFile, writeFile, mkdir} from 'node:fs/promises';

export interface GroPluginDenoCompileOptions {
	/**
	 * Entry point TypeScript file.
	 */
	entry: string;

	/**
	 * Output binary name (without path).
	 */
	output_name: string;

	/**
	 * Output directory for the binary.
	 * @default 'dist_cli'
	 */
	output_dir?: string;

	/**
	 * Deno permissions to grant.
	 * @default ['--allow-all']
	 */
	permissions?: Array<string>;

	/**
	 * Additional deno compile flags.
	 * @default []
	 */
	flags?: Array<string>;

	/**
	 * Generate a SHA-256 hash file alongside the binary.
	 * The hash file is named `{output_name}.sha256` and uses sha256sum format.
	 * @default true
	 */
	generate_hash?: boolean;
}

/**
 * Creates a gro plugin that compiles TypeScript to a binary using `deno compile`.
 * Runs during the `adapt` phase (production build only).
 */
export const gro_plugin_deno_compile = (options: GroPluginDenoCompileOptions): Plugin => {
	const {
		entry,
		output_name,
		output_dir = './dist_cli',
		permissions = ['--allow-all'],
		flags = [],
		generate_hash = true,
	} = options;

	return {
		name: 'gro_plugin_deno_compile',
		adapt: async ({log, timings}) => {
			const timing = timings.start('deno compile');

			// Ensure output directory exists
			await mkdir(output_dir, {recursive: true});

			// Extract imports from root deno.json and write minimal config to output dir.
			// This avoids bundling all npm devDependencies from package.json.
			// Relative paths are rebased to account for the output subdirectory.
			const root_deno_json = JSON.parse(await readFile('deno.json', 'utf-8'));
			const rebased_imports: Record<string, string> = {};
			for (const [key, value] of Object.entries(root_deno_json.imports as Record<string, string>)) {
				if (value.startsWith('./') || value.startsWith('../')) {
					const trailing_slash = value.endsWith('/');
					const rebased = join('..', value);
					rebased_imports[key] = rebased + (trailing_slash ? '/' : '');
				} else {
					rebased_imports[key] = value;
				}
			}
			const cli_deno_json = {imports: rebased_imports};
			const cli_config_path = join(output_dir, 'deno.json');
			await writeFile(cli_config_path, JSON.stringify(cli_deno_json, null, '\t') + '\n');
			log.info(`[gro_plugin_deno_compile] created ${cli_config_path}`);

			// Compile from output_dir to avoid package.json detection
			const entry_relative = relative(output_dir, entry);
			const args = [
				'compile',
				...permissions,
				'--config',
				'deno.json',
				'--output',
				output_name,
				...flags,
				entry_relative,
			];

			log.info(`[gro_plugin_deno_compile] compiling ${entry} → ${join(output_dir, output_name)}`);

			const result = await spawn('deno', args, {cwd: output_dir});

			if (!result.ok) {
				throw new TaskError(
					`deno compile failed (${spawn_result_to_message(result)}): ${entry} → ${output_dir}/${output_name}`,
				);
			}

			const output_path = join(output_dir, output_name);
			log.info(`[gro_plugin_deno_compile] binary created: ${output_path}`);

			// Generate SHA-256 hash file for verification
			if (generate_hash) {
				const binary = await readFile(output_path);
				const hash = await hash_sha256(binary);
				const hash_path = `${output_path}.sha256`;
				// Use sha256sum format: "<hash>  <filename>"
				await writeFile(hash_path, `${hash}  ${output_name}\n`);
				log.info(`[gro_plugin_deno_compile] hash created: ${hash_path}`);
			}

			timing();
		},
	};
};
