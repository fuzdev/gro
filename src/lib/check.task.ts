import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';
import { plural } from '@fuzdev/fuz_util/string.ts';
import { styleText as st } from 'node:util';

import { TaskError, to_task_command, type Task } from './task.ts';
import { to_forwarded_args_by_command, to_forwarded_argv, to_raw_rest_args } from './args.ts';
import { VITEST_CLI } from './constants.ts';
import { is_fs_error } from './fs_error.ts';
import { package_json_sync } from './package_json.ts';
import { fs } from './fs.ts';
import { create_fs_paths, type FsPaths } from './fs_paths.ts';
import { GIT_LOCAL_PASS_ENV, git_check_clean_workspace } from './git.ts';
import { spawn } from './spawn.ts';
import { process } from './process.ts';
import { paths, to_root_path } from './paths.ts';
import { find_test_files, project_has_vitest } from './vitest_helpers.ts';
import type { FsSearchResult } from './fs_search.ts';

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
	lint: s.with_default(meta(s.boolean, { description: 'dual of no-lint' }), true),
	'no-lint': s.with_default(meta(s.boolean, { description: 'opt out of linting' }), false),
	build: s.with_default(meta(s.boolean, { description: 'dual of no-build' }), false),
	'no-build': s.with_default(meta(s.boolean, { description: 'opt out of building' }), true),
	force_build: s.with_default(
		meta(s.boolean, { description: 'force a fresh build, ignoring the cache' }),
		false
	),
	sync: s.with_default(meta(s.boolean, { description: 'dual of no-sync' }), true),
	'no-sync': s.with_default(meta(s.boolean, { description: 'opt out of syncing' }), false),
	install: s.with_default(meta(s.boolean, { description: 'opt into installing packages' }), false),
	workspace: s.with_default(
		meta(s.boolean, {
			description: 'ensure a clean git workspace, useful for CI, also implies --no-sync'
		}),
		false
	)
});
export type Args = s.Infer<typeof Args>;

/** @nodocs */
export const task: Task<Args> = {
	summary: 'check that everything is ready to commit',
	Args,
	run: async ({ args, invoke_task, log, config }) => {
		const {
			typecheck,
			test,
			gen,
			format,
			package_json,
			lint,
			build,
			force_build,
			sync,
			install,
			workspace
		} = args;
		const fs_paths = create_fs_paths(fs);

		// When checking the workspace, which was added for CI, never sync.
		// Setup like installing packages and `sveltekit-sync` should be done in the CI setup.
		if (!workspace) {
			if (sync || install) {
				if (!sync) log.warn('sync is false but install is true, so ignoring the sync option');
				await invoke_task('sync', { install, gen: false }); // never generate because `gro gen --check` runs below
			}
		}

		// Build before typechecking and tests so there can be a dependency
		if (build) {
			// Skip sync/gen/install since check handles those separately
			await invoke_task('build', { sync: false, gen: false, install: false, force_build });
		}

		if (typecheck) {
			await invoke_task('typecheck');
		}

		if (test) {
			// forwarded args for the tests ask for them, so the test task decides
			if (has_forwarded_test_args(fs_paths) || (await project_has_vitest(fs_paths, log))) {
				await invoke_task('test');
			} else {
				const test_files = await find_test_files_or_throw(fs_paths);
				if (test_files.length > 0) throw new TaskError(to_undeclared_vitest_message(test_files));
				log.info('vitest is not declared and no test files were found; skipping tests');
			}
		}

		if (gen) {
			await invoke_task('gen', { check: true });
		}

		if (package_json && config.map_package_json) {
			const { changed } = await package_json_sync(fs_paths, config.map_package_json, log, false);
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
			await invoke_task('lint');
		}

		// Disallow TODO*.md files in the project root on CI.
		if (process.env.get('CI') === 'true') {
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
			// only reads the local repo, so the named set is all it needs
			const git = spawn.find('git', { pass_env: GIT_LOCAL_PASS_ENV });
			if (!git) {
				// no git means nothing to compare against, the same as a directory that's no repo
				log.warn(st('yellow', 'git not found on PATH, skipping the workspace check'));
			} else {
				const error_message = await git_check_clean_workspace(git);
				if (error_message) {
					log.error(st('red', 'git status'));
					await git.run(['status']);
					throw new TaskError(
						'Failed check for git_check_clean_workspace:' +
							error_message +
							' - do you need to run `gro sync` or commit some files?'
					);
				}
			}
		}
	}
};

// a `-- gro test` section, which can name another `dir`, or args forwarded to vitest
const has_forwarded_test_args = (fs_paths: FsPaths): boolean => {
	const raw_rest_args = to_raw_rest_args(process.args);
	return (
		Object.hasOwn(
			to_forwarded_args_by_command(raw_rest_args, fs_paths.project.path),
			to_task_command('test')
		) || to_forwarded_argv(VITEST_CLI, raw_rest_args).length > 0
	);
};

const find_test_files_or_throw = async (fs_paths: FsPaths): Promise<Array<FsSearchResult>> => {
	try {
		return await find_test_files(fs_paths, paths.source);
	} catch (error) {
		if (!is_fs_error(error)) throw error;
		throw new TaskError(
			`failed to search ${to_root_path(paths.source)} for test files, as vitest isn't declared: ` +
				error.message,
			{ cause: error }
		);
	}
};

const to_undeclared_vitest_message = (test_files: Array<FsSearchResult>): string => {
	const count = test_files.length;
	const printed = test_files.slice(0, TEST_FILES_PRINTED_MAX).map((f) => to_root_path(f.id));
	const unprinted = count - printed.length;
	return (
		`found ${count} test file${plural(count)} but vitest isn't declared in package.json: ` +
		printed.join(', ') +
		(unprinted > 0 ? ` and ${unprinted} more` : '') +
		' - add vitest to devDependencies, remove the test files, or pass --no-test'
	);
};
