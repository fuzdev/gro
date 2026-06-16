import type {Logger} from '@fuzdev/fuz_util/log.ts';
import {spawn_process, spawn_result_to_message} from '@fuzdev/fuz_util/process.ts';
import {styleText as st} from 'node:util';

import {TaskError} from './task.ts';

/**
 * Env vars that tell npm to skip devDependencies. We strip these from the
 * install subprocess env because gro's install step exists to make the project
 * buildable — and the build needs devDeps (TypeScript, Vite, svelte-kit's
 * adapter packages, etc.). Inheriting `NODE_ENV=production` from the shell
 * prunes those packages and then `gro build` fails on the very next step with
 * an opaque "Cannot find package" error.
 *
 * Users who genuinely want a production-pruned install run it themselves
 * (e.g. `npm ci --omit=dev`) outside the gro pipeline.
 */
const PRUNE_TRIGGERING_ENV_VARS: ReadonlySet<string> = new Set([
	'NODE_ENV',
	'npm_config_production',
	'npm_config_omit',
]);

/**
 * Returns a copy of `env` with the devDependency-pruning vars removed.
 *
 * @param env - the source environment to sanitize
 * @returns a fresh env object; the input is not mutated
 */
export const sanitize_install_env = (env: NodeJS.ProcessEnv): NodeJS.ProcessEnv => {
	const result: NodeJS.ProcessEnv = {};
	for (const [key, value] of Object.entries(env)) {
		if (PRUNE_TRIGGERING_ENV_VARS.has(key)) continue;
		result[key] = value;
	}
	return result;
};

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
	/** short human-readable outcome, e.g. `code 1` or `error: spawn ENOENT` */
	detail: string;
}

/**
 * Runs an npm-compatible command and reports `ok` plus captured stderr. Injected
 * by `install_with_cache_healing` for testing; the default implementation
 * spawns a real subprocess.
 */
export type NpmCommandRunner = (
	command: string,
	args: ReadonlyArray<string>,
	options?: {cwd?: string; env?: NodeJS.ProcessEnv},
) => Promise<NpmCommandResult>;

/**
 * Default runner: streams the subprocess live while capturing stderr so an
 * ETARGET-class failure can be detected. `stdin`/`stdout` inherit (live npm
 * output); `stderr` is teed to `process.stderr` and accumulated.
 */
const run_npm_command: NpmCommandRunner = async (command, args, options) => {
	const {child, closed} = spawn_process(command, args, {
		...options,
		stdio: ['inherit', 'inherit', 'pipe'],
	});
	let stderr = '';
	child.stderr?.on('data', (data: Buffer) => {
		stderr += data.toString();
		process.stderr.write(data);
	});
	const result = await closed;
	return {ok: result.ok, stderr, detail: spawn_result_to_message(result)};
};

/**
 * Options for `install_with_cache_healing`.
 */
export interface InstallCacheHealingOptions {
	/** working directory for the install; defaults to the current directory */
	cwd?: string;
	/** base env; devDependency-pruning vars are stripped via `sanitize_install_env` */
	env?: NodeJS.ProcessEnv;
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
 * The env is always passed through `sanitize_install_env` so the build's
 * devDependencies survive.
 *
 * @param pm_cli - the npm-compatible CLI to run (e.g. `config.pm_cli`)
 * @param options - cwd, env, logger, extra install args, and an injectable runner
 * @returns the result of the (possibly retried) install
 */
export const install_with_cache_healing = async (
	pm_cli: string,
	options: InstallCacheHealingOptions = {},
): Promise<NpmCommandResult> => {
	const {cwd, env = process.env, log, run = run_npm_command, install_args = []} = options;
	const install_env = sanitize_install_env(env);
	const install_options = {cwd, env: install_env};
	const install_command_args = ['install', ...install_args];

	const first = await run(pm_cli, install_command_args, install_options);
	if (first.ok) return first;

	// Heal only npm's ETARGET-class failures; any other failure (or non-npm pm)
	// is surfaced as-is, since the cache-clean heal below is npm-specific.
	if (pm_cli !== 'npm' || !is_etarget_error(first.detail, first.stderr)) {
		return first;
	}

	log?.warn(
		st('yellow', `ETARGET detected — running \`${pm_cli} cache clean --force\` and retrying`),
	);

	const cache_clean = await run(pm_cli, ['cache', 'clean', '--force'], install_options);
	if (!cache_clean.ok) {
		return {
			ok: false,
			stderr: cache_clean.stderr,
			detail: `\`${pm_cli} cache clean --force\` failed: ${cache_clean.detail}`,
		};
	}

	const retry = await run(pm_cli, install_command_args, install_options);
	if (retry.ok) {
		log?.info(st('green', 'dependencies installed after cache heal'));
	}
	return retry;
};

/**
 * `install_with_cache_healing` that throws a `TaskError` on failure instead of
 * returning the result — the shape every task callsite wants. The thrown
 * message names the exact command (including `install_args`), an optional
 * `context` clause, and the failure `detail`.
 *
 * @param pm_cli - the npm-compatible CLI to run (e.g. `config.pm_cli`)
 * @param options - same as `install_with_cache_healing`, plus an optional
 *   `context` clause appended after the command (e.g. `'after version bump'`)
 * @throws TaskError when the install fails, even after a cache-heal retry
 */
export const install_with_cache_healing_or_throw = async (
	pm_cli: string,
	options: InstallCacheHealingOptions & {context?: string} = {},
): Promise<void> => {
	const result = await install_with_cache_healing(pm_cli, options);
	if (result.ok) return;
	const command = ['install', ...(options.install_args ?? [])].join(' ');
	const context = options.context ? ` ${options.context}` : '';
	const detail = result.detail ? `: ${result.detail}` : '';
	throw new TaskError(`Failed \`${pm_cli} ${command}\`${context}${detail}`);
};
