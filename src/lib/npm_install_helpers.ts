import type { Logger } from '@fuzdev/fuz_util/log.ts';
import { styleText as st } from 'node:util';
import { dirname, join } from 'node:path';

import { TaskError } from './task.ts';
import { gro_paths } from './paths.ts';
import { spawn_result_is_ok, to_spawn_result_message } from './cli.ts';
import type { Spawn, SpawnProgram, SpawnProgramOptions } from './spawn.ts';
import { is_spawn_error } from './spawn_error.ts';
import type { FsReadDir } from './fs.ts';
import { package_json_load } from './package_json.ts';
import type { FsPaths } from './fs_paths.ts';

/**
 * Env vars that tell npm to skip devDependencies. We strip these from the
 * install subprocess env because gro's install step exists to make the project
 * buildable — and the build needs devDeps (TypeScript, Vite, svelte-kit's
 * adapter packages, etc.). Inheriting `NODE_ENV=production` from the shell
 * prunes those packages and then `gro build` fails on the very next step with
 * an opaque "Cannot find package" error.
 *
 * Users who genuinely want a production-pruned install run it themselves
 * (e.g. `npm ci --omit=dev`) outside the Gro pipeline.
 */
const PRUNE_TRIGGERING_ENV_VARS: ReadonlyArray<string> = [
	'NODE_ENV',
	'npm_config_production',
	'npm_config_omit'
];

/**
 * The options of the program that runs an install: every host variable but the pruning ones.
 *
 * The pruning names are absent, not set to empty, as an empty `NODE_ENV` is a different value to
 * a lifecycle script, and the host's `LC_ALL` stays exactly as it is, unset included,
 * since an empty one changes the locale a Node lifecycle script gets.
 *
 * @returns the options for `spawn.program`, given on a root since a narrowed one refuses `'all'`
 */
export const to_install_program_options = (): SpawnProgramOptions => ({
	pass_env: 'all',
	omit_env: PRUNE_TRIGGERING_ENV_VARS
});

/**
 * Checks if an npm install failure is caused by stale cache (ETARGET).
 *
 * Detects the several shapes npm uses for "the requested version isn't there":
 * `code ETARGET`, `ETARGET`, `notarget`, and `No matching version found`.
 *
 * @param message - the short outcome string (e.g. `code 1`)
 * @param stderr - the captured stderr of the failed command
 * @returns `true` when the failure looks like an ETARGET-class staleness error
 */
export const is_etarget_error = (message: string, stderr: string): boolean => {
	const combined = `${message} ${stderr}`.toLowerCase();
	return (
		combined.includes('etarget') ||
		combined.includes('notarget') ||
		combined.includes('no matching version found')
	);
};

/**
 * Result of one install or cache-clean attempt.
 */
export interface NpmCommandResult {
	ok: boolean;
	/** captured stderr; the default runner also tees this live to the terminal */
	stderr: string;
	/** short human-readable outcome, e.g. `code 1` or the `SpawnError` message */
	detail: string;
}

/**
 * Runs an npm-compatible command and reports `ok` plus captured stderr. Injected
 * by `install_with_cache_healing` for testing; the default implementation
 * spawns a real subprocess.
 */
export type NpmCommandRunner = (
	program: SpawnProgram,
	args: ReadonlyArray<string>,
	options?: { cwd?: FsReadDir }
) => Promise<NpmCommandResult>;

/**
 * Default runner: streams the subprocess live while capturing stderr so an
 * ETARGET-class failure can be detected. `stdin`/`stdout` inherit (live npm
 * output); `stderr` is teed to `process.stderr` and accumulated.
 */
const run_npm_command: NpmCommandRunner = async (program, args, options) => {
	let child;
	try {
		child = await program.start(args, { ...options, stdio: ['inherit', 'inherit', 'pipe'] });
	} catch (error) {
		if (is_spawn_error(error)) return { ok: false, stderr: '', detail: error.message };
		throw error;
	}
	const decoder = new TextDecoder();
	let stderr = '';
	for await (const chunk of child.stderr!) {
		stderr += decoder.decode(chunk, { stream: true });
		process.stderr.write(chunk);
	}
	stderr += decoder.decode();
	const result = await child.closed;
	return {
		ok: spawn_result_is_ok(result),
		stderr,
		detail: to_spawn_result_message(result)
	};
};

/**
 * Options for `install_with_cache_healing`.
 */
export interface InstallCacheHealingOptions {
	/** working directory for the install; defaults to the project root */
	cwd?: FsReadDir;
	/** logger for cache-heal progress */
	log?: Logger;
	/** injectable command runner for tests; defaults to a live-teeing spawn */
	run?: NpmCommandRunner;
	/**
	 * Extra args appended after `install`, e.g. package specs and flags for
	 * `gro upgrade` (`['foo@latest', '--force']`) or a single dep for
	 * `gro changeset` (`['-D', '@changesets/changelog-git']`). The same args are
	 * reused on the post-cache-clean retry.
	 */
	install_args?: ReadonlyArray<string>;
}

/**
 * Installs dependencies, self-healing the stale-cache (ETARGET) failure mode.
 *
 * **Strategy:**
 * 1. First attempt: `${pm_cli} install [...install_args]`.
 * 2. On an npm ETARGET-class failure (stale cache): `npm cache clean --force`
 *    then retry the same install once.
 * 3. On any other failure — or any failure under a non-npm `pm_cli` — return
 *    immediately without healing.
 *
 * **Why ETARGET happens:** right after a dependency is published and the
 * registry has propagated, npm's local cache may still hold stale
 * "404"/no-version metadata for it; clearing the cache forces a fresh metadata
 * fetch. This makes installs safe to run immediately after publishing an
 * upstream package — e.g. picking up a just-released dep in a
 * dependency-ordered publish chain, or `gro upgrade`-ing to a fresh `@latest`.
 *
 * **npm only:** the heal uses npm-specific syntax (`cache clean --force`), so
 * it's gated on `pm_cli === 'npm'`. A non-npm `pm_cli` surfaces the failure
 * unchanged rather than running a command it can't understand.
 *
 * The env is always `to_install_program_options` so the build's devDependencies survive,
 * and the program is pinned once, for the install and its retry.
 *
 * @param spawn - a root, which resolves `pm_cli` on `PATH` and takes the install's environment
 * @param pm_cli - the npm-compatible CLI to run (e.g. `config.pm_cli`)
 * @param options - cwd, logger, extra install args, and an injectable runner
 * @returns the result of the (possibly retried) install
 */
export const install_with_cache_healing = async (
	spawn: Spawn,
	pm_cli: string,
	options: InstallCacheHealingOptions = {}
): Promise<NpmCommandResult> => {
	const { cwd, log, run = run_npm_command, install_args = [] } = options;
	let program: SpawnProgram;
	try {
		program = spawn.program(pm_cli, to_install_program_options());
	} catch (error) {
		if (is_spawn_error(error)) return { ok: false, stderr: '', detail: error.message };
		throw error;
	}
	const install_options = { cwd };
	const install_command_args = ['install', ...install_args];

	const first = await run(program, install_command_args, install_options);
	if (first.ok) return first;

	// Heal only npm's ETARGET-class failures; any other failure (or non-npm pm)
	// is surfaced as-is, since the cache-clean heal below is npm-specific.
	if (pm_cli !== 'npm' || !is_etarget_error(first.detail, first.stderr)) {
		return first;
	}

	log?.warn(
		st('yellow', `ETARGET detected — running \`${pm_cli} cache clean --force\` and retrying`)
	);

	const cache_clean = await run(program, ['cache', 'clean', '--force'], install_options);
	if (!cache_clean.ok) {
		return {
			ok: false,
			stderr: cache_clean.stderr,
			detail: `\`${pm_cli} cache clean --force\` failed: ${cache_clean.detail}`
		};
	}

	const retry = await run(program, install_command_args, install_options);
	if (retry.ok) {
		log?.info(st('green', 'dependencies installed after cache heal'));
	}
	return retry;
};

/**
 * Installed versions of the running Gro and the packages it depends on, keyed
 * by package name, with `null` for a package that isn't installed.
 */
export type GroPackageVersions = Map<string, string | null>;

/**
 * Reads the installed version of `name` as Node would resolve it from `from_dir`,
 * searching each `node_modules` up to the filesystem root.
 */
const installed_version_load = async (
	fs_paths: FsPaths,
	name: string,
	from_dir: string
): Promise<string | null> => {
	let dir = from_dir;
	for (;;) {
		// an unreadable package.json loads as `PACKAGE_JSON_EMPTY`, with an empty version
		const { version } = await package_json_load(
			fs_paths,
			join(dir, 'node_modules', name),
			undefined,
			false
		);
		if (version) return version;
		const parent = dirname(dir);
		if (parent === dir) return null;
		dir = parent;
	}
};

/**
 * Reads the installed versions of the Gro package at `gro_dir` and its
 * dependencies and peer dependencies, as they sit in `node_modules`. These are
 * the packages a running Gro has loaded, so comparing snapshots from before and
 * after an install tells whether the process is now stale.
 * Read from disk, not through the module system, whose cache keeps what it first loaded.
 *
 * @param fs_paths - opens the `package.json` files, beneath the project or not
 * @param gro_dir - the Gro package directory; defaults to the running Gro's
 * @param names - more packages to read beside the ones Gro's `package.json` names,
 *   so an after-install snapshot covers the deps an upgraded Gro dropped
 * @returns the versions keyed by package name, `@fuzdev/gro` included
 */
export const gro_package_versions_load = async (
	fs_paths: FsPaths,
	gro_dir: string = gro_paths.root,
	names: Iterable<string> = []
): Promise<GroPackageVersions> => {
	const versions: GroPackageVersions = new Map();
	const gro_package_json = await package_json_load(fs_paths, gro_dir, undefined, false);
	versions.set('@fuzdev/gro', gro_package_json.version || null);
	const dep_names = new Set([
		...Object.keys(gro_package_json.dependencies ?? {}),
		...Object.keys(gro_package_json.peerDependencies ?? {}),
		...names
	]);
	dep_names.delete('@fuzdev/gro');
	await Promise.all(
		Array.from(dep_names, async (name) => {
			versions.set(name, await installed_version_load(fs_paths, name, gro_dir));
		})
	);
	return versions;
};

/**
 * Lists the packages whose installed version differs between two snapshots,
 * formatted as `name before → after` and sorted by name.
 *
 * @param before - the snapshot from before the install
 * @param after - the snapshot from after the install
 * @returns one line per changed package, empty when nothing changed
 */
export const gro_package_versions_diff = (
	before: GroPackageVersions,
	after: GroPackageVersions
): Array<string> => {
	const changed: Array<string> = [];
	for (const name of new Set([...before.keys(), ...after.keys()])) {
		const a = before.get(name) ?? null;
		const b = after.get(name) ?? null;
		if (a !== b) changed.push(`${name} ${a ?? 'none'} → ${b ?? 'none'}`);
	}
	return changed.sort();
};

/**
 * Options for `install_with_cache_healing_or_throw`,
 * with `fs_paths` exactly when `throw_if_gro_packages_changed` is set.
 */
export type InstallOrThrowOptions = InstallCacheHealingOptions & {
	/** clause appended after the command in the error, e.g. `'after version bump'` */
	context?: string;
} & (
		| { throw_if_gro_packages_changed?: false; fs_paths?: undefined; gro_dir?: undefined }
		| {
				/**
				 * Throw a `TaskError` when the install changed the installed version of Gro
				 * or any of its dependencies. Set it when the task keeps running in this
				 * process after installing (invoking other tasks, running gen), since this
				 * process has the old versions loaded, and modules it imports afterward come
				 * from the new ones, a mix that can fail in confusing ways. Rerunning the
				 * command starts a process that loads only the new versions.
				 */
				throw_if_gro_packages_changed: true;
				/** reads the installed versions, as `gro_package_versions_load` does */
				fs_paths: FsPaths;
				/** the Gro package directory to check; defaults to the running Gro's, for tests */
				gro_dir?: string;
		  }
	);

/**
 * `install_with_cache_healing` that throws a `TaskError` on failure instead of
 * returning the result — the shape every task callsite wants. The thrown
 * message names the exact command (including `install_args`), an optional
 * `context` clause, and the failure `detail`.
 *
 * @param spawn - a root, which resolves `pm_cli` on `PATH`
 * @param pm_cli - the npm-compatible CLI to run (e.g. `config.pm_cli`)
 * @param options - same as `install_with_cache_healing`, plus the error's
 *   `context` clause and the `throw_if_gro_packages_changed` check
 * @throws TaskError when the install fails, even after a cache-heal retry, or
 *   with `throw_if_gro_packages_changed`, when it changed Gro or its dependencies
 */
export const install_with_cache_healing_or_throw = async (
	spawn: Spawn,
	pm_cli: string,
	options: InstallOrThrowOptions = {}
): Promise<void> => {
	const { gro_dir } = options;
	const fs_paths = options.throw_if_gro_packages_changed ? options.fs_paths : null;
	const before = fs_paths ? await gro_package_versions_load(fs_paths, gro_dir) : null;
	const result = await install_with_cache_healing(spawn, pm_cli, options);
	const command = ['install', ...(options.install_args ?? [])].join(' ');
	if (!result.ok) {
		const context = options.context ? ` ${options.context}` : '';
		const detail = result.detail ? `: ${result.detail}` : '';
		throw new TaskError(`Failed \`${pm_cli} ${command}\`${context}${detail}`);
	}
	if (!before || !fs_paths) return;
	const after = await gro_package_versions_load(fs_paths, gro_dir, before.keys());
	const changed = gro_package_versions_diff(before, after);
	if (changed.length === 0) return;
	throw new TaskError(
		`\`${pm_cli} ${command}\` updated Gro or its dependencies, which this process already loaded,` +
			` so re-run the command to continue with the new versions:\n  ${changed.join('\n  ')}`
	);
};
