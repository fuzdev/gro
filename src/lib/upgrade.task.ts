import { basename, dirname, resolve } from 'node:path';
import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';

import { to_forwarded_argv, to_raw_rest_args } from './task_args.ts';
import { process } from './process.ts';
import { find_cli, spawn_cli, spawn_result_is_ok, to_spawn_result_message } from './cli.ts';
import { install_with_cache_healing_or_throw } from './npm_install_helpers.ts';
import { NODE_MODULES_DIRNAME } from './constants.ts';
import { GitOrigin, git_pull, to_git_program } from './git.ts';
import {
	package_json_extract_dependencies,
	package_json_load,
	type PackageJsonDep
} from './package_json.ts';
import { TaskError, type Task } from './task.ts';
import { fs } from './fs.ts';
import { create_fs_paths, type FsPaths } from './fs_paths.ts';
import { spawn } from './spawn.ts';

/** @nodocs */
export const Args = s.object({
	_: s.with_default_fn(
		meta(s.array(s.string), { description: 'names of deps to exclude from the upgrade' }),
		() => []
	),
	// the default is outside the pipe, already an array, so it skips the transform
	only: s.with_default_fn(
		s.pipe(
			meta(s.union([s.string, s.array(s.string)]), {
				description: 'names of deps to include in the upgrade'
			}),
			s.transform((v: string | Array<string>) => (Array.isArray(v) ? v : [v]))
		),
		() => []
	),
	origin: s.with_default(
		meta(GitOrigin, { description: 'git origin to pull from before upgrading' }),
		'origin'
	),
	force: s.with_default(
		meta(s.boolean, { description: 'if true, passes --force to the package manager install' }),
		false
	),
	pull: s.with_default(meta(s.boolean, { description: 'dual of no-pull' }), true),
	'no-pull': s.with_default(meta(s.boolean, { description: 'opt out of git pull' }), false),
	delete_node_modules: s.with_default(
		meta(s.boolean, { description: 'if true, deletes node_modules before upgrading' }),
		false
	),
	// TODO maybe configured globally instead
	node_modules_path: s.with_default(
		meta(s.string, { description: 'path to modules directory to delete' }),
		NODE_MODULES_DIRNAME
	),
	delete_lockfile: s.with_default(
		meta(s.boolean, { description: 'if true, deletes the lockfile before upgrading' }),
		false
	),
	lockfile_path: s.with_default(
		meta(s.string, { description: 'path to the lockfile to delete' }),
		'package-lock.json'
	),
	dry: s.with_default(
		meta(s.boolean, { description: 'if true, print out the planned upgrades' }),
		false
	)
});
export type Args = s.Infer<typeof Args>;

/** @nodocs */
export const task: Task<Args> = {
	summary: 'upgrade deps',
	Args,
	run: async ({ task_args, log, config }): Promise<void> => {
		const {
			_,
			only,
			origin,
			force,
			pull,
			delete_node_modules,
			node_modules_path,
			delete_lockfile,
			lockfile_path,
			dry
		} = task_args;

		if (_.length && only.length) {
			throw new TaskError('Cannot call `gro upgrade` with both rest args and --only.');
		}

		const fs_paths = create_fs_paths(fs);

		// TODO maybe a different task that pulls and does other things, like `gro ready`
		if (pull && !dry) {
			// reaches the remote and the user's credential helpers, so the whole environment
			await git_pull(to_git_program(spawn, { pass_env: 'all' }), origin);
		}

		if (delete_node_modules) {
			log.info(
				dry ? 'dry run, keeping node_modules at' : 'deleting node_modules at',
				node_modules_path
			);
			const entry = await to_project_entry(fs_paths, node_modules_path, '--node_modules_path');
			if (!dry) await fs_paths.project.remove(entry, { recursive: true, missing_ok: true });
		}

		if (delete_lockfile) {
			log.info(dry ? 'dry run, keeping lockfile at' : 'deleting lockfile at', lockfile_path);
			const entry = await to_project_entry(fs_paths, lockfile_path, '--lockfile_path');
			if (!dry) await fs_paths.project.remove(entry, { missing_ok: true });
		}

		const package_json = await package_json_load(fs_paths);

		const all_deps = package_json_extract_dependencies(package_json);

		const deps = only.length
			? all_deps.filter((d) => only.includes(d.name))
			: all_deps.filter((d) => !_.includes(d.name));

		if (only.length && only.length !== deps.length) {
			throw new TaskError(
				`Some deps to upgrade were not found: ${only
					.filter((o) => !deps.find((d) => d.name === o))
					.join(', ')}`
			);
		}

		const upgrade_items = to_upgrade_items(deps);

		log.info(`upgrading:`, upgrade_items.join(' '));

		const install_args = [...upgrade_items];
		if (dry) {
			install_args.push('--dry-run');
			log.info(`deps`, deps);
		}
		if (force) {
			install_args.push('--force');
		}
		install_args.push(...to_forwarded_argv(config.pm_cli, to_raw_rest_args(process.args)));
		await install_with_cache_healing_or_throw(spawn, config.pm_cli, {
			install_args,
			heal_cache: !dry,
			log,
			stderr: process.stderr
		});

		// TODO @many this relies on npm behavior that changed in v11
		// If we deleted the lockfile or node modules, `npm install` again
		// to fix a recurring npm bug getting the lockfile to its final state.
		if (!dry && (delete_node_modules || delete_lockfile)) {
			log.info(`installing again to fix npm lockfile bugs`);
			await install_with_cache_healing_or_throw(spawn, config.pm_cli, {
				log,
				stderr: process.stderr
			});
		}

		if (dry) return;

		// Sync in a new process to pick up any changes after installing, avoiding some errors.
		const gro = find_cli(spawn, 'gro', { pass_env: 'all' });
		if (gro) {
			const sync_result = await spawn_cli(gro, ['sync']); // don't install because we do above
			if (!spawn_result_is_ok(sync_result)) {
				throw new TaskError(
					`the upgrade is installed, but \`gro sync\` failed: ${to_spawn_result_message(sync_result)}`
				);
			}
		}
	}
};

/**
 * The entry a path to delete names, beneath the project: its parent resolved with symlinks
 * followed must be the project or inside it, and the entry itself, a symlink removed as
 * itself, can't be the project. Anything else is refused, so these flags never delete
 * outside the project.
 */
const to_project_entry = async (fs_paths: FsPaths, path: string, flag: string): Promise<string> => {
	const resolved = resolve(fs_paths.project.path, path);
	const name = basename(resolved);
	const parent = name ? await fs_paths.open_path(dirname(resolved)) : null;
	if (parent === null || parent.dir !== fs_paths.project) {
		throw new TaskError(
			`\`${flag} ${path}\` resolves outside the project, and \`gro upgrade\` deletes only inside it`
		);
	}
	return parent.path ? parent.path + '/' + name : name;
};

const EXACT_VERSION_MATCHER = /^..*@.+/;
const CUSTOM_TAG_MATCHER = /^[\^~><=]*.+-(.+)/;

// TODO hacky and limited
// TODO probably want to pass through exact deps as well, e.g. @foo/bar@1
const to_upgrade_items = (deps: Array<PackageJsonDep>): Array<string> =>
	deps.map((dep) => {
		if (EXACT_VERSION_MATCHER.test(dep.name)) {
			return dep.name;
		}
		const custom_tag_matches = CUSTOM_TAG_MATCHER.exec(dep.version);
		if (custom_tag_matches) {
			return dep.name + '@' + custom_tag_matches[1]!.split('.')[0]; // I tried adding `\.?` to the end but doesn't work and I'm being lazy so I'm just splitting
		}
		return dep.name + '@latest';
	});
