import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';
import { plural } from '@fuzdev/fuz_util/string.ts';
import { st } from '@fuzdev/fuz_util/print.ts';
import type { Logger } from '@fuzdev/fuz_util/log.ts';

import { to_task_command } from './task.ts';
import { TaskError } from './task_error.ts';
import { to_forwarded_args_by_command, to_forwarded_argv } from './task_args.ts';
import { ESLINT_CLI, LOCKFILE_FILENAME, VITEST_CLI } from './constants.ts';
import { SVELTEKIT_CLI_GRANT, sveltekit_sync_if_obviously_needed } from './sveltekit_helpers.ts';
import { npm_install_check, type NpmInstallMismatch } from './npm_install_check.ts';
import { find_eslint_config } from './eslint_helpers.ts';
import { is_fs_error } from './fs_error.ts';
import { package_json_sync, project_has_dependency } from './package_json.ts';
import type { FsPaths } from './fs_paths.ts';
import {
	GIT_LOCAL_PASS_ENV,
	git_check_workspace,
	git_workspace_is_clean,
	git_workspace_status_message
} from './git.ts';
import type { Env } from './env.ts';
import type { GroConfig } from './gro_config.ts';
import { create_paths, to_root_path, type Paths } from './paths.ts';
import { find_test_files } from './vitest_helpers.ts';
import type { FsSearchResult } from './fs_search.ts';
import { format_error_detail } from './error_detail.ts';
import type {
	BuiltinInvokes,
	BuiltinInvokeTask,
	BuiltinPrograms,
	BuiltinTask,
	BuiltinTaskGrants
} from './builtin_host.ts';

const TEST_FILES_PRINTED_MAX = 5;

/** @nodocs */
export const Args = s.object({
	typecheck: s.with_default(meta(s.boolean, { description: 'dual of no-typecheck' }), true),
	'no-typecheck': s.with_default(
		meta(s.boolean, { description: 'opt out of typechecking' }),
		false
	),
	test: s.with_default(meta(s.boolean, { description: 'dual of no-test' }), true),
	'no-test': s.with_default(meta(s.boolean, { description: 'opt out of running tests' }), false),
	gen: s.with_default(meta(s.boolean, { description: 'dual of no-gen' }), true),
	'no-gen': s.with_default(meta(s.boolean, { description: 'opt out of gen check' }), false),
	format: s.with_default(meta(s.boolean, { description: 'dual of no-format' }), true),
	'no-format': s.with_default(meta(s.boolean, { description: 'opt out of format check' }), false),
	package_json: s.with_default(meta(s.boolean, { description: 'dual of no-package_json' }), true),
	'no-package_json': s.with_default(
		meta(s.boolean, { description: 'opt out of package.json check' }),
		false
	),
	lint: s.with_default(meta(s.boolean, { description: 'opt into linting' }), false),
	build: s.with_default(meta(s.boolean, { description: 'dual of no-build' }), false),
	'no-build': s.with_default(meta(s.boolean, { description: 'opt out of building' }), true),
	force_build: s.with_default(
		meta(s.boolean, { description: 'force a fresh build, ignoring the cache' }),
		false
	),
	lockfile: s.with_default(meta(s.boolean, { description: 'dual of no-lockfile' }), true),
	'no-lockfile': s.with_default(
		meta(s.boolean, { description: 'opt out of checking the install against package-lock.json' }),
		false
	),
	install: s.with_default(
		meta(s.boolean, { description: 'opt into installing packages first, with gro sync' }),
		false
	),
	workspace: s.with_default(
		meta(s.boolean, {
			description: 'ensure a clean git workspace, useful for CI, also skipping --install'
		}),
		false
	)
});
export type Args = s.Infer<typeof Args>;

const grants = {
	// reads `package.json`, the lockfiles and installed packages, `src/` for test files,
	// and the root's listing for `TODO*.md`, writing none of them
	fs: { read: ['.'] },
	env: ['CI'],
	programs: {
		// `--workspace` reads the local repo's status
		git_cli: { name: 'git', pass_env: GIT_LOCAL_PASS_ENV },
		// syncs SvelteKit's files when they're missing, before `build`, whose `svelte-package` reads them;
		// `svelte-kit sync` loads the project's Vite config, code whose environment Gro can't know
		sveltekit_cli: SVELTEKIT_CLI_GRANT
	},
	invokes: {
		// `--install` installs, which nothing else in `check` does
		sync: { when: 'install' },
		// a plain `check` holds no build in its closure
		build: { when: 'build' },
		// each step the command line can turn off, held only while it's on
		typecheck: { when: 'typecheck' },
		test: { when: 'test' },
		// `gen --check` and `format --check`, writing no source
		gen: { when: 'gen', check: true },
		format: { when: 'format', check: true },
		lint: { when: 'lint' }
	}
} as const satisfies BuiltinTaskGrants;

/** @nodocs */
export const task: BuiltinTask<Args, typeof grants> = {
	summary: 'check that everything is ready to commit',
	Args,
	grants,
	run: ({ task_args, config, raw_rest_args, invoke_task, log, host }) =>
		run_check(
			task_args,
			{ map_package_json: config.map_package_json, raw_rest_args },
			{ fs_paths: host.fs_paths, env: host.env, programs: host.programs, invoke_task, log }
		)
};

/**
 * The data `run_check` takes beside its args.
 */
export interface CheckOptions {
	map_package_json: GroConfig['map_package_json'];
	/**
	 * The CLI args from the first `--` on, whose `gro test` or `vitest` section asks for the tests,
	 * and whose `gro lint` or `eslint` section, given with `--lint`, asks for the linter.
	 */
	raw_rest_args: ReadonlyArray<string>;
}

/**
 * What `run_check` reaches, picked from a builtin's `host` and context.
 */
export interface CheckDeps {
	fs_paths: FsPaths;
	env: Pick<Env, 'get'>;
	programs: Pick<BuiltinPrograms<'git_cli' | 'sveltekit_cli'>, 'find'>;
	invoke_task: BuiltinInvokeTask<BuiltinInvokes<typeof grants>>;
	log: Logger;
}

/**
 * Checks that everything is ready to commit, `gro check`'s body.
 * @param args - the task's parsed args
 * @param options - the config's `map_package_json` and the forwarded args
 * @param deps - the project's boundary, `CI`, git and `svelte-kit`, the nested tasks, and the log
 */
export const run_check = async (
	args: Args,
	options: CheckOptions,
	deps: CheckDeps
): Promise<void> => {
	const { map_package_json, raw_rest_args } = options;
	const { fs_paths, env, programs, invoke_task, log } = deps;
	const {
		typecheck,
		test,
		gen,
		format,
		package_json,
		lint,
		build,
		force_build,
		lockfile,
		install,
		workspace
	} = args;
	const paths = create_paths(fs_paths.project.path);

	// linting is opt-in, so forwarded args for the linter without `--lint` fail before any step,
	// rather than being dropped
	const forwarded_lint_args = await has_forwarded_lint_args(raw_rest_args, fs_paths);
	if (forwarded_lint_args && !lint) {
		throw new TaskError(
			'args are forwarded to the linter, which `gro check` runs only with --lint' +
				' - pass --lint or drop the `-- gro lint` or `-- eslint` section'
		);
	}

	// `check` writes neither the install nor `package.json`, verifying both below,
	// but for `--install`, which installs first, as CI's setup does instead under `--workspace`;
	// `gro gen --check` and the `package.json` check run below, so the sync runs neither
	if (install && !workspace) {
		await invoke_task('sync', { install, gen: false, package_json: false });
	}

	if (lockfile) {
		const checked = await npm_install_check(fs_paths);
		if (!checked.checked) {
			log.info(`the install wasn't checked: ${checked.reason}`);
		} else {
			if (checked.linked.length) {
				log.info(
					`the install check skipped the linked packages, which follow their source: ` +
						checked.linked.join(', ')
				);
			}
			if (checked.mismatches.length) {
				throw new TaskError(to_install_mismatch_message(checked.mismatches));
			}
			log.info('check passed for the install');
		}
	}

	// Build before typechecking and tests so there can be a dependency
	if (build) {
		// `build`'s `svelte-package` reads SvelteKit's files, which nothing in `check`
		// has synced by then, and `build` is told not to sync
		await sveltekit_sync_if_obviously_needed(
			fs_paths,
			{ find: () => programs.find('sveltekit_cli') },
			log
		);
		// Skip sync/gen/install since check handles those separately
		await invoke_task('build', { sync: false, gen: false, install: false, force_build });
	}

	if (typecheck) {
		await invoke_task('typecheck');
	}

	if (test) {
		// forwarded args for the tests ask for them, so the test task decides
		if (
			(await has_forwarded_test_args(raw_rest_args, fs_paths)) ||
			(await project_has_dependency(fs_paths, VITEST_CLI, log))
		) {
			// the typecheck synced SvelteKit already
			await invoke_task('test', typecheck ? { sync: false } : {});
		} else {
			const test_files = await find_test_files_or_throw(fs_paths, paths);
			if (test_files.length > 0)
				throw new TaskError(to_undeclared_vitest_message(test_files, paths));
			log.info('vitest is not declared and no test files were found; skipping tests');
		}
	}

	if (gen) {
		await invoke_task('gen', { check: true });
	}

	if (package_json && map_package_json) {
		const { changed } = await package_json_sync(fs_paths, map_package_json, log, false);
		if (changed) {
			throw new TaskError('package.json is out of date, run `gro sync` to update it');
		} else {
			log.info('check passed for package.json');
		}
	}

	if (format) {
		await invoke_task('format', { check: true });
	}

	// Run the linter last to surface every other kind of problem first.
	// It's not the ideal order when the linter would catch errors that cause failing tests,
	// but it's better for most usage.
	if (lint) {
		// forwarded args for the linter ask for it, so the lint task decides
		if (forwarded_lint_args || (await project_has_dependency(fs_paths, ESLINT_CLI, log))) {
			await invoke_task('lint');
		} else {
			const eslint_config = await find_eslint_config_or_throw(fs_paths);
			if (eslint_config !== null) throw new TaskError(to_undeclared_eslint_message(eslint_config));
			log.info('eslint is not declared and the project root has no ESLint config; skipping lint');
		}
	}

	// Disallow TODO*.md files in the project root on CI.
	if (env.get('CI') === 'true') {
		const todo_files: Array<string> = [];
		for await (const { name } of fs_paths.project.list()) {
			if (name.startsWith('TODO') && name.endsWith('.md')) todo_files.push(name);
		}
		todo_files.sort();
		if (todo_files.length > 0) {
			throw new TaskError(
				'Found disallowed TODO*.md files in project root: ' + todo_files.join(', ')
			);
		}
	}

	if (workspace) {
		// a workspace check that was asked for and can't run fails, as it would otherwise pass unchecked
		// only reads the local repo, so the named set is all it needs
		const git = programs.find('git_cli');
		if (!git) {
			throw new TaskError(
				'--workspace checks the git workspace and needs git, which was not found on PATH' +
					' - install git or drop --workspace'
			);
		}
		const status = await git_check_workspace(git);
		if (!status) {
			throw new TaskError(
				"--workspace checks the git workspace, and this directory isn't in a git repo" +
					' - run it in the repo or drop --workspace'
			);
		}
		if (!git_workspace_is_clean(status)) {
			log.error(st('red', 'git status'));
			await git.run(['status']);
			throw new TaskError(
				`--workspace checks the git workspace, and ${git_workspace_status_message(status)}` +
					(status.untracked_files
						? ' - commit them or remove the untracked files'
						: ' - commit them') +
					", or run `gro sync` if they're generated"
			);
		}
	}
};

// a `-- gro test` section, which can name another `dir`, or args forwarded to vitest
const has_forwarded_test_args = async (
	raw_rest_args: ReadonlyArray<string>,
	fs_paths: FsPaths
): Promise<boolean> =>
	Object.hasOwn(
		await to_forwarded_args_by_command(raw_rest_args, fs_paths),
		to_task_command('test')
	) || to_forwarded_argv(VITEST_CLI, raw_rest_args).length > 0;

// a `-- gro lint` section, or args forwarded to ESLint
const has_forwarded_lint_args = async (
	raw_rest_args: ReadonlyArray<string>,
	fs_paths: FsPaths
): Promise<boolean> =>
	Object.hasOwn(
		await to_forwarded_args_by_command(raw_rest_args, fs_paths),
		to_task_command('lint')
	) || to_forwarded_argv(ESLINT_CLI, raw_rest_args).length > 0;

const find_eslint_config_or_throw = async (fs_paths: FsPaths): Promise<string | null> => {
	try {
		return await find_eslint_config(fs_paths);
	} catch (error) {
		if (!is_fs_error(error)) throw error;
		throw new TaskError(
			"failed to look for an ESLint config, as eslint isn't declared: " +
				format_error_detail(error),
			{ cause: error }
		);
	}
};

const INSTALL_MISMATCHES_PRINTED_MAX = 10;

const to_install_mismatch_message = (mismatches: Array<NpmInstallMismatch>): string => {
	const count = mismatches.length;
	const printed = mismatches.slice(0, INSTALL_MISMATCHES_PRINTED_MAX).map((m) => m.message);
	const unprinted = count - printed.length;
	return (
		`the install doesn't match package.json and ${LOCKFILE_FILENAME}: run \`npm i\`, ` +
		`or pass \`--no-lockfile\` to skip this check: ` +
		printed.join('; ') +
		(unprinted > 0 ? `; and ${unprinted} more` : '')
	);
};

const to_undeclared_eslint_message = (eslint_config: string): string =>
	`found ${eslint_config} but eslint isn't declared in package.json` +
	` - add eslint to devDependencies, remove ${eslint_config}, or drop --lint`;

const find_test_files_or_throw = async (
	fs_paths: FsPaths,
	paths: Paths
): Promise<Array<FsSearchResult>> => {
	try {
		return await find_test_files(fs_paths, paths.source);
	} catch (error) {
		if (!is_fs_error(error)) throw error;
		throw new TaskError(
			`failed to search ${to_root_path(paths.source, paths.root)} for test files, as vitest isn't declared: ` +
				format_error_detail(error),
			{ cause: error }
		);
	}
};

const to_undeclared_vitest_message = (test_files: Array<FsSearchResult>, paths: Paths): string => {
	const count = test_files.length;
	const printed = test_files
		.slice(0, TEST_FILES_PRINTED_MAX)
		.map((f) => to_root_path(f.id, paths.root));
	const unprinted = count - printed.length;
	return (
		`found ${count} test file${plural(count)} but vitest isn't declared in package.json: ` +
		printed.join(', ') +
		(unprinted > 0 ? ` and ${unprinted} more` : '') +
		' - add vitest to devDependencies, remove the test files, or pass --no-test'
	);
};
