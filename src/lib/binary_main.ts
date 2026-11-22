/**
 * The body of the binary's CLI root, as `invoke.ts` is the Node Gro's: made for the entry the
 * binary generates for a command to call with the host modules every command's run grants.
 *
 * Such an entry imports those rows and this module and makes one call. This module is Gro's
 * embedded code, whose origin holds no grant, so it imports no granted host module itself:
 * it builds the `FsPaths`, the project loader, and the binary's `GroPlatform` from what the entry
 * hands it, runs `gro_main`, and reports what that throws with `log_cli_error`, setting the
 * exit code from `to_exit_code` rather than exiting, so the run ends on its own, leaving the
 * binary to print its own report after.
 *
 * @module
 */

import { gro_main, log_cli_error, to_exit_code } from './gro_main.ts';
import { create_fs_paths } from './fs_paths.ts';
import { TaskError } from './task_error.ts';
import type { Env } from './env.ts';
import type { Fs } from './fs.ts';
import type { Spawn } from './spawn.ts';
import type { ProcessOutput } from './stdout.ts';
import type { GroPlatform } from './invoke_task.ts';
import type { ModuleGraph } from './module_graph.ts';
import { to_file_url } from './internal/posix_path.ts';

/**
 * What the binary's generated entry hands `binary_main`: the host modules it imported, as they
 * export them, and the values the binary read natively, written into the entry's text.
 * It mixes data with capabilities on purpose, as `GroMainDeps` does: the entry builds the
 * whole set for one run, and splitting it would add a parameter and separate nothing.
 */
export interface BinaryMainDeps {
	/**
	 * The CLI args after the binary's own path, its grant flags already split off,
	 * the args host module's `args`.
	 */
	readonly args: ReadonlyArray<string>;
	/**
	 * The fs host module's root, the project its `project`.
	 */
	readonly fs: Fs;
	/**
	 * The env host module's `env`.
	 */
	readonly env: Env;
	/**
	 * The spawn host module's `spawn`.
	 */
	readonly spawn: Spawn;
	/**
	 * The stdout host module's `stdout`.
	 */
	readonly stdout: ProcessOutput;
	/**
	 * The stderr host module's `stderr`.
	 */
	readonly stderr: ProcessOutput;
	/**
	 * The cwd host module's `cwd`, the working directory, which the `FsPaths` refuse to remove.
	 */
	readonly cwd: () => string;
	/**
	 * The exit host module's `set_exit_code`, which takes the code of a failed run.
	 */
	readonly set_exit_code: (code: number) => void;
	/**
	 * Imports a project module by its `file:` URL, `(url) => import(url)` in the entry, so the
	 * import resolves from the project's origin, where Gro's embedded code may import no file.
	 */
	readonly import_url: (url: string) => Promise<Record<string, any>>;
	/**
	 * `HOME`, which the binary reads and the `FsPaths` refuse to remove, absent when it's unset.
	 */
	readonly home: string | undefined;
	/**
	 * The binary's version, every task's context's `gro.version`.
	 */
	readonly version: string;
}

/**
 * Runs the Gro CLI on `deps.args` with what the binary's entry hands it, reporting a failure
 * and setting the exit code from it.
 * @throws only when reading what `gro_main` threw throws, as from an `exit_code` getter that throws,
 * which leaves the entry's rejection to fail the run
 */
export const binary_main = async (deps: BinaryMainDeps): Promise<void> => {
	const { args, fs, env, spawn, stdout, stderr, import_url } = deps;
	try {
		await gro_main({
			args,
			fs_paths: create_fs_paths(fs, { cwd: deps.cwd(), home: deps.home }),
			host_root: { env, spawn, stdout, stderr },
			import_module: (id) => import_url(to_file_url(id)),
			platform: create_binary_platform(to_command_word(args), deps.version)
		});
	} catch (error) {
		log_cli_error(error, 'gro');
		deps.set_exit_code(to_exit_code(error));
	}
};

/**
 * The binary's platform: the running Gro has no package directory, and the module graph and the
 * Svelte config, read through Node's machinery, refuse naming the command and its Node run.
 * @param command - the command word, which the refusals name
 * @param version - the binary's version
 */
const create_binary_platform = (command: string | undefined, version: string): GroPlatform => {
	const refuse = (what: string): Promise<never> => {
		const gro_command = command === undefined ? 'gro' : `gro ${command}`;
		return Promise.reject(
			new TaskError(
				`\`${gro_command}\` needs ${what}, which only the Node gro has: ` +
					`run \`node_modules/.bin/${gro_command}\``
			)
		);
	};
	const graph: ModuleGraph = {
		modules: () => refuse('the module graph'),
		dependents: () => refuse('the module graph'),
		watch: () => refuse('the module graph'),
		// it never loads, so it closes without loading
		close: () => Promise.resolve()
	};
	return {
		create_module_graph: () => graph,
		load_svelte_config: () => refuse('the Svelte config'),
		gro: { version, dir: null }
	};
};

// the command word, the first positional arg before any `--`, none for the listing
const to_command_word = (args: ReadonlyArray<string>): string | undefined => {
	for (const arg of args) {
		if (arg === '--') return undefined;
		if (!arg.startsWith('-')) return arg;
	}
	return undefined;
};
