import {readdir} from 'node:fs/promises';
import {z} from 'zod';
import {spawn} from '@fuzdev/fuz_util/process.js';
import {styleText as st} from 'node:util';
import {git_check_clean_workspace} from '@fuzdev/fuz_util/git.js';

import {TaskError, type Task} from './task.ts';
import {package_json_sync} from './package_json.ts';

/** @nodocs */
export const Args = z.strictObject({
	typecheck: z.boolean().meta({description: 'dual of no-typecheck'}).default(true),
	'no-typecheck': z.boolean().meta({description: 'opt out of typechecking'}).default(false),
	test: z.boolean().meta({description: 'dual of no-test'}).default(true),
	'no-test': z.boolean().meta({description: 'opt out of running tests'}).default(false),
	gen: z.boolean().meta({description: 'dual of no-gen'}).default(true),
	'no-gen': z.boolean().meta({description: 'opt out of gen check'}).default(false),
	format: z.boolean().meta({description: 'dual of no-format'}).default(true),
	'no-format': z.boolean().meta({description: 'opt out of format check'}).default(false),
	package_json: z.boolean().meta({description: 'dual of no-package_json'}).default(true),
	'no-package_json': z
		.boolean()
		.meta({description: 'opt out of package.json check'})
		.default(false),
	lint: z.boolean().meta({description: 'dual of no-lint'}).default(true),
	'no-lint': z.boolean().meta({description: 'opt out of linting'}).default(false),
	build: z.boolean().meta({description: 'dual of no-build'}).default(false),
	'no-build': z.boolean().meta({description: 'opt out of building'}).default(true),
	force_build: z
		.boolean()
		.meta({description: 'force a fresh build, ignoring the cache'})
		.default(false),
	sync: z.boolean().meta({description: 'dual of no-sync'}).default(true),
	'no-sync': z.boolean().meta({description: 'opt out of syncing'}).default(false),
	install: z.boolean().meta({description: 'opt into installing packages'}).default(false),
	workspace: z
		.boolean()
		.meta({description: 'ensure a clean git workspace, useful for CI, also implies --no-sync'})
		.default(false),
});
export type Args = z.infer<typeof Args>;

/** @nodocs */
export const task: Task<Args> = {
	summary: 'check that everything is ready to commit',
	Args,
	run: async ({args, invoke_task, log, config}) => {
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
			workspace,
		} = args;

		// When checking the workspace, which was added for CI, never sync.
		// Setup like installing packages and `sveltekit-sync` should be done in the CI setup.
		if (!workspace) {
			if (sync || install) {
				if (!sync) log.warn('sync is false but install is true, so ignoring the sync option');
				await invoke_task('sync', {install, gen: false}); // never generate because `gro gen --check` runs below
			}
		}

		// Build before typechecking and tests so there can be a dependency
		if (build) {
			// Skip sync/gen/install since check handles those separately
			await invoke_task('build', {sync: false, gen: false, install: false, force_build});
		}

		if (typecheck) {
			await invoke_task('typecheck');
		}

		if (test) {
			await invoke_task('test');
		}

		if (gen) {
			await invoke_task('gen', {check: true});
		}

		if (package_json && config.map_package_json) {
			const {changed} = await package_json_sync(config.map_package_json, log, false);
			if (changed) {
				throw new TaskError('package.json is out of date, run `gro sync` to update it');
			} else {
				log.info('check passed for package.json');
			}
		}

		if (format) {
			await invoke_task('format', {check: true});
		}

		// Run the linter last to surface every other kind of problem first.
		// It's not the ideal order when the linter would catch errors that cause failing tests,
		// but it's better for most usage.
		if (lint) {
			await invoke_task('lint');
		}

		// Disallow TODO*.md files in the project root on CI.
		if (process.env.CI === 'true') {
			const root_files = await readdir('.');
			const todo_files = root_files.filter((f) => f.startsWith('TODO') && f.endsWith('.md'));
			if (todo_files.length > 0) {
				throw new TaskError(
					'Found disallowed TODO*.md files in project root: ' + todo_files.join(', '),
				);
			}
		}

		if (workspace) {
			const error_message = await git_check_clean_workspace();
			if (error_message) {
				log.error(st('red', 'git status'));
				await spawn('git', ['status']);
				throw new TaskError(
					'Failed check for git_check_clean_workspace:' +
						error_message +
						' - do you need to run `gro sync` or commit some files?',
				);
			}
		}
	},
};
