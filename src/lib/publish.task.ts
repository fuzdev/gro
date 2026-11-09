import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';
import { styleText as st } from 'node:util';

import { TaskError, type Task } from './task.ts';
import {
	GitBranch,
	GitOrigin,
	git_check_clean_workspace,
	git_checkout,
	git_fetch,
	git_head_subject,
	git_pull,
	git_tag_exists,
	to_git_program
} from './git.ts';
import { install_with_cache_healing_or_throw } from './npm_install_helpers.ts';
import { package_json_load, package_json_parse_repo_url } from './package_json.ts';
import { find_cli, spawn_cli, spawn_result_is_ok } from './cli.ts';
import { has_sveltekit_library, sveltekit_sync } from './sveltekit_helpers.ts';
import { update_changelog } from './changelog.ts';
import { load_from_env } from './env.ts';
import { CHANGESET_CLI, changeset_has_pending } from './changeset_helpers.ts';
import { fs } from './fs.ts';
import { create_fs_paths } from './fs_paths.ts';
import { spawn } from './spawn.ts';
import { process } from './process.ts';

/** @nodocs */
export const Args = s.object({
	branch: s.with_default(meta(GitBranch, { description: 'branch to publish from' }), 'main'),
	origin: s.with_default(meta(GitOrigin, { description: 'git origin to publish from' }), 'origin'),
	changelog: s.with_default(
		meta(s.string, { description: 'file name and path of the changelog' }),
		'CHANGELOG.md'
	),
	preserve_changelog: s.with_default(
		meta(s.boolean, {
			description:
				'opt out of linkifying and formatting the changelog from @changesets/changelog-git'
		}),
		false
	),
	optional: s.with_default(
		meta(s.boolean, { description: 'exit gracefully if there are no changesets' }),
		false
	),
	dry: s.with_default(
		meta(s.boolean, { description: 'build and prepare to publish without actually publishing' }),
		false
	),
	check: s.with_default(meta(s.boolean, { description: 'dual of no-check' }), true),
	'no-check': s.with_default(
		meta(s.boolean, { description: 'opt out of checking before publishing' }),
		false
	),
	build: s.with_default(meta(s.boolean, { description: 'dual of no-build' }), true),
	'no-build': s.with_default(meta(s.boolean, { description: 'opt out of building' }), false),
	pull: s.with_default(meta(s.boolean, { description: 'dual of no-pull' }), true),
	'no-pull': s.with_default(meta(s.boolean, { description: 'opt out of git pull' }), false),
	sync: s.with_default(meta(s.boolean, { description: 'dual of no-sync' }), true),
	'no-sync': s.with_default(meta(s.boolean, { description: 'opt out of gro sync' }), false),
	install: s.with_default(meta(s.boolean, { description: 'dual of no-install' }), true),
	'no-install': s.with_default(
		meta(s.boolean, { description: 'opt out of installing packages before publishing' }),
		false
	),
	force_build: s.with_default(
		meta(s.boolean, { description: 'force a fresh build, ignoring the cache' }),
		false
	),
	changeset_cli: s.with_default(
		meta(s.string, { description: 'the changeset CLI to use' }),
		CHANGESET_CLI
	)
});
export type Args = s.Infer<typeof Args>;

const to_release_commit_subject = (version: string): string => `publish v${version}`;

/** @nodocs */
export const task: Task<Args> = {
	summary: 'bump version, publish to the configured registry, and git push',
	Args,
	run: async ({ args, log, invoke_task, config }): Promise<void> => {
		const {
			branch,
			origin,
			changelog,
			preserve_changelog,
			dry,
			check,
			build,
			pull,
			sync,
			install,
			force_build,
			optional,
			changeset_cli
		} = args;
		if (dry) {
			log.info(st('green', 'dry run!'));
		}

		const fs_paths = create_fs_paths(fs);

		const package_json = await package_json_load(fs_paths);

		const has_sveltekit_library_result = await has_sveltekit_library(fs_paths, package_json);
		if (!has_sveltekit_library_result.ok) {
			throw new TaskError(
				'Failed to find SvelteKit library: ' + has_sveltekit_library_result.message
			);
		}

		const changelog_file = await fs_paths.open_path(changelog);
		const changelog_exists = await changelog_file.dir.exists(changelog_file.path);

		const found_changeset_cli = find_cli(spawn, changeset_cli, { pass_env: 'all' });
		if (!found_changeset_cli) {
			throw new TaskError(
				'changeset command not found, install @changesets/cli locally or globally'
			);
		}

		// a named unconfined phase, `-A`-equivalent: it pushes and signs with the user's git setup
		const git = to_git_program(spawn, { pass_env: 'all' });

		// Make sure we're on the right branch:
		await git_fetch(git, origin, branch);
		await git_checkout(git, branch);
		if (pull) {
			if (await git_check_clean_workspace(git)) {
				throw new TaskError('The git workspace is not clean, pass --no-pull to bypass git pull');
			}
			await git_pull(git, origin, branch);
		}

		// Install packages to ensure deps are current.
		// Handles cases like branch switches where package.json changed.
		// Skip gen because it will run after version bump.
		if (sync || install) {
			if (!sync) log.warn('sync is false but install is true, so running sync for install only');
			await invoke_task('sync', { install, gen: false });
		}

		// Check before proceeding, defaults to true.
		if (check) {
			await invoke_task('check', { workspace: true, sync: false });
		}

		let version!: string;
		let nothing_to_publish = false;

		// before the `dry` branch, so a dry run refuses what a real publish would
		if (typeof package_json.version !== 'string') {
			throw new TaskError('Failed to find package.json version');
		}
		const parsed_repo_url = package_json_parse_repo_url(package_json);
		if (!parsed_repo_url) {
			throw new TaskError(
				'package.json `repository` must contain a repo url (and GitHub only for now, sorry),' +
					' like `git+https://github.com/fuzdev/gro.git` or `https://github.com/fuzdev/gro`' +
					' or an object with the `url` key'
			);
		}

		// Changesets tags only after npm takes the version, and the release commit comes first,
		// so a release commit without its tag is a publish that stopped short of npm
		const resuming =
			(await git_head_subject(git)) === to_release_commit_subject(package_json.version) &&
			!(await git_tag_exists(git, `v${package_json.version}`));
		const has_changesets = resuming || (await changeset_has_pending(fs_paths.project));
		// before the `dry` branch, so a dry run refuses what a real publish would
		if (!has_changesets && !optional) {
			throw new TaskError('no changesets to publish, add one with `gro changeset`');
		}

		const undo_command = 'git reset --hard HEAD~1';
		const resume_command = 'gro publish';
		let release_committed = false;

		// Bump and commit before building, so the build sees a clean workspace
		// and the tag `changeset publish` puts on `HEAD` names the source that was built.
		if (dry) {
			log.info('dry run, skipping changeset version');
		} else if (resuming) {
			version = package_json.version;
			release_committed = true;
			log.info(`resuming the release of v${version}, its commit has no tag`);
		} else if (!has_changesets) {
			nothing_to_publish = true;
			log.info('no changesets to publish');
		} else {
			// This is the first line that alters the repo.

			const changeset_version_result = await spawn_cli(found_changeset_cli, ['version'], log);
			if (!spawn_result_is_ok(changeset_version_result)) {
				throw new TaskError('changeset version failed: no commits were made: see the error above');
			}

			if (!preserve_changelog) {
				const token = await load_from_env(
					fs_paths,
					process.env.narrow(['SECRET_GITHUB_API_TOKEN']).get_all(),
					'SECRET_GITHUB_API_TOKEN',
					// one key, named here, so a token in a workspace's `.env` serves its repos
					['.env', '../.env']
				);
				if (!token) {
					log.warn(
						'the env var SECRET_GITHUB_API_TOKEN was not found in the env, `.env`, or `../.env`,' +
							' so API calls will be unauthorized'
					);
				}
				await update_changelog(
					parsed_repo_url.owner,
					parsed_repo_url.repo,
					globalThis.fetch,
					fs_paths,
					changelog,
					token,
					log
				);
			}

			// Update package-lock.json to reflect the new version.
			if (install) {
				await install_with_cache_healing_or_throw(spawn, config.pm_cli, {
					log,
					stderr: process.stderr,
					context: 'after version bump'
				});
				// npm prunes SvelteKit's generated `node_modules/$app` (not a package) on install,
				// so re-sync before gen reads a tsconfig that extends `$app/tsconfig`.
				await sveltekit_sync(spawn, { pm_cli: config.pm_cli });
			}

			// Regenerate files that depend on package.json version.
			// The check above ensures gen is updated.
			await invoke_task('gen');

			const package_json_after_versioning = await package_json_load(fs_paths);
			version = package_json_after_versioning.version!;
			if (package_json.version === version) {
				// The version didn't change.
				// For now this is the best detection we have for a no-op `changeset version`.
				if (optional) {
					nothing_to_publish = true;
				} else {
					// Doesn't build if the version didn't change and publishing isn't optional.
					throw new TaskError(`\`${changeset_cli} version\` failed: are there any changes?`);
				}
			} else {
				if (!changelog_exists && (await changelog_file.dir.exists(changelog_file.path))) {
					await git.run(['add', '--', changelog]);
				}
				const commit_result = await git.run([
					'commit',
					'-a',
					'-m',
					to_release_commit_subject(version)
				]);
				if (!spawn_result_is_ok(commit_result)) {
					throw new TaskError(
						`the release commit failed, and v${version} is not published - resolve the failure` +
							' and publish again, after undoing the version bump with `git reset --hard`'
					);
				}
				release_committed = true;
			}
		}

		// Build after the version is bumped so the new version is in the build as needed.
		// Skip sync and install because we already handled both above,
		// and gen once the release is committed, because its output has to be in that commit.
		if (build) {
			try {
				await invoke_task('build', {
					sync: false,
					install: false,
					gen: !release_committed,
					force_build
				});
			} catch (error) {
				if (release_committed) {
					log.error(
						`the build failed, and v${version} is not published - fix it and run` +
							` \`${resume_command}\` to resume, or undo the release with \`${undo_command}\``
					);
				}
				throw error;
			}
		}

		// Return early if there are no changes and publishing is optional, but after building,
		// so if callers want to optimize away building
		// they need to do so manually like in `gro release`.
		// TODO this could be cleaned up if tasks had a return value to callers, it could specifiy that it didn't build
		if (nothing_to_publish) return;

		if (dry) {
			log.info('publishing branch ' + branch);
			log.info(st('green', 'dry run complete!'));
			return;
		}

		const push_command = `git push --follow-tags ${origin} ${branch}`;
		const changeset_publish_result = await spawn_cli(found_changeset_cli, ['publish'], log);
		if (!spawn_result_is_ok(changeset_publish_result)) {
			throw new TaskError(
				`\`${changeset_cli} publish\` failed - fix it and run \`${resume_command}\` to resume,` +
					` or undo the release with \`${undo_command}\``
			);
		}

		// `branch` is checked out above, so this pushes the release commit
		const push_result = await git.run([
			'push',
			'--follow-tags',
			'--end-of-options',
			origin,
			branch
		]);
		if (!spawn_result_is_ok(push_result)) {
			throw new TaskError(
				`\`${push_command}\` failed: the release commit and tag v${version} are local only,` +
					` but v${version} is already published to the registry - resolve the failure,` +
					` then push manually with \`${push_command}\``
			);
		}

		log.info(st('green', `published to branch ${st('cyan', branch)}!`));
	}
};
