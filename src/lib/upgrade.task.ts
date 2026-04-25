import {args_serialize} from '@fuzdev/fuz_util/args.js';
import {GitOrigin, git_pull} from '@fuzdev/fuz_util/git.js';
import {spawn} from '@fuzdev/fuz_util/process.js';
import {rm} from 'node:fs/promises';
import {z} from 'zod';

import {to_forwarded_args} from './args.ts';
import {spawn_cli} from './cli.ts';
import {NODE_MODULES_DIRNAME} from './constants.ts';
import {
	package_json_extract_dependencies,
	package_json_load,
	type PackageJsonDep,
} from './package_json.ts';
import {TaskError, type Task} from './task.ts';

/** @nodocs */
export const Args = z.strictObject({
	_: z
		.array(z.string())
		.meta({description: 'names of deps to exclude from the upgrade'})
		.default([]),
	only: z
		.union([z.string(), z.array(z.string())])
		.meta({
			description: 'names of deps to include in the upgrade',
		})
		.default([])
		.transform((v) => (Array.isArray(v) ? v : [v])),
	origin: GitOrigin.meta({description: 'git origin to deploy to'}).default('origin'),
	force: z.boolean().meta({description: 'if true, print out the planned upgrades'}).default(false),
	pull: z.boolean().meta({description: 'dual of no-pull'}).default(true),
	'no-pull': z.boolean().meta({description: 'opt out of git pull'}).default(false),
	delete_node_modules: z
		.boolean()
		.meta({description: 'if true, deletes node_modules before upgrading'})
		.default(false),
	node_modules_path: z // TODO maybe configured globally instead
		.string()
		.meta({description: 'path to modules directory to delete'})
		.default(NODE_MODULES_DIRNAME),
	delete_lockfile: z
		.boolean()
		.meta({description: 'if true, deletes the lockfile before upgrading'})
		.default(false),
	lockfile_path: z
		.string()
		.meta({description: 'path to the lockfile to delete'})
		.default('package-lock.json'),
	dry: z.boolean().meta({description: 'if true, print out the planned upgrades'}).default(false),
});
export type Args = z.infer<typeof Args>;

/** @nodocs */
export const task: Task<Args> = {
	summary: 'upgrade deps',
	Args,
	run: async ({args, log, config}): Promise<void> => {
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
			dry,
		} = args;

		if (_.length && only.length) {
			throw new TaskError('Cannot call `gro upgrade` with both rest args and --only.');
		}

		// TODO maybe a different task that pulls and does other things, like `gro ready`
		if (pull) {
			await git_pull(origin);
		}

		if (delete_node_modules) {
			log.info(`deleting node_modules at `, node_modules_path);
			await rm(node_modules_path, {recursive: true, force: true});
		}

		if (delete_lockfile) {
			log.info(`deleting lockfile at`, lockfile_path);
			await rm(lockfile_path, {force: true});
		}

		const package_json = await package_json_load();

		const all_deps = package_json_extract_dependencies(package_json);

		const deps = only.length
			? all_deps.filter((d) => only.includes(d.name))
			: all_deps.filter((d) => !_.includes(d.name));

		if (only.length && only.length !== deps.length) {
			throw new TaskError(
				`Some deps to upgrade were not found: ${only.filter((o) => !deps.find((d) => d.name === o)).join(', ')}`,
			);
		}

		const upgrade_items = to_upgrade_items(deps);

		log.info(`upgrading:`, upgrade_items.join(' '));

		const install_args = ['install'].concat(upgrade_items);
		if (dry) {
			install_args.push('--dry-run');
			log.info(`deps`, deps);
		}
		if (force) {
			install_args.push('--force');
		}
		install_args.push(...args_serialize(to_forwarded_args(config.pm_cli)));
		await spawn(config.pm_cli, install_args);

		// TODO @many this relies on npm behavior that changed in v11
		// If we deleted the lockfile or node modules, `npm install` again
		// to fix a recurring npm bug getting the lockfile to its final state.
		if (!dry && (delete_node_modules || delete_lockfile)) {
			log.info(`installing again to fix npm lockfile bugs`);
			await spawn(config.pm_cli, ['install']);
		}

		// Sync in a new process to pick up any changes after installing, avoiding some errors.
		await spawn_cli('gro', ['sync']); // don't install because we do above
	},
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
