import {z} from 'zod';
import {spawn} from '@fuzdev/fuz_util/process.js';
import {styleText as st} from 'node:util';
import type {WrittenConfig} from '@changesets/types';
import {readdir, readFile, writeFile} from 'node:fs/promises';
import {join} from 'node:path';
import {fs_exists} from '@fuzdev/fuz_util/fs.js';
import {
	GitOrigin,
	git_check_fully_staged_workspace,
	git_push_to_create,
} from '@fuzdev/fuz_util/git.js';

import {TaskError, type Task} from './task.ts';
import {find_cli, spawn_cli} from './cli.ts';
import {has_sveltekit_library} from './sveltekit_helpers.ts';
import {
	CHANGESET_CLI,
	CHANGESET_DIR,
	ChangesetAccess,
	ChangesetBump,
	CHANGESET_PUBLIC_ACCESS,
	CHANGESET_RESTRICTED_ACCESS,
} from './changeset_helpers.ts';
import {package_json_load} from './package_json.ts';

/** @nodocs */
export const Args = z.strictObject({
	/**
	 * The optional rest args get joined with a space to form the `message`.
	 */
	_: z
		.array(z.string())
		.meta({description: 'the message for the changeset and commit'})
		.max(1)
		.default([]),
	minor: z.boolean().meta({description: 'bump the minor version'}).default(false),
	major: z.boolean().meta({description: 'bump the major version'}).default(false),
	dir: z.string().meta({description: 'changeset dir'}).default(CHANGESET_DIR),
	access: ChangesetAccess.meta({
		description: "changeset 'access' config value, the default depends on package.json#private",
	}).optional(),
	changelog: z
		.string()
		.meta({description: 'changelog dep package name, used as changeset\'s "changelog" config'})
		.default('@changesets/changelog-git'),
	dep: z.boolean().meta({description: 'dual of no-dep'}).default(true),
	'no-dep': z
		.boolean()
		.meta({description: 'opt out of installing the changelog package'})
		.default(false),
	origin: GitOrigin.meta({description: 'git origin to deploy to'}).default('origin'),
	changeset_cli: z.string().meta({description: 'the changeset CLI to use'}).default(CHANGESET_CLI),
});
export type Args = z.infer<typeof Args>;

/**
 * Calls the `changeset` CLI with some simple automations.
 * This API is designed for convenient manual usage, not clarity or normality.
 *
 * Usage:
 * - gro changeset some commit message
 * - gro changeset some commit message --minor
 * - gro changeset "some commit message" --minor
 *
 * @nodocs
 */
export const task: Task<Args> = {
	summary: 'call changeset with gro patterns',
	Args,
	run: async (ctx): Promise<void> => {
		const {
			invoke_task,
			args: {
				_: [message],
				minor,
				major,
				dir,
				access: access_arg,
				changelog,
				dep,
				origin,
				changeset_cli,
			},
			log,
			svelte_config,
			config,
		} = ctx;

		if (!message && (minor || major)) throw new TaskError('cannot bump version without a message');
		if (minor && major) throw new TaskError('cannot bump both minor and major');

		const bump: ChangesetBump = minor ? 'minor' : major ? 'major' : 'patch';

		const found_changeset_cli = await find_cli(changeset_cli);
		if (!found_changeset_cli) {
			throw new TaskError(
				'changeset command not found: install @changesets/cli locally or globally',
			);
		}

		const package_json = await package_json_load();

		const has_sveltekit_library_result = await has_sveltekit_library(package_json, svelte_config);
		if (!has_sveltekit_library_result.ok) {
			throw new TaskError(
				'Failed to find SvelteKit library: ' + has_sveltekit_library_result.message,
			);
		}

		const path = join(dir, 'config.json');

		const inited = await fs_exists(path);

		if (!inited) {
			await spawn_cli(found_changeset_cli, ['init'], log);

			const access =
				access_arg ??
				(package_json.private ? CHANGESET_RESTRICTED_ACCESS : CHANGESET_PUBLIC_ACCESS);

			const access_color = access === CHANGESET_RESTRICTED_ACCESS ? 'blue' : 'red';
			log.info('initing changeset with ' + st(access_color, access) + ' access');
			if (access !== CHANGESET_RESTRICTED_ACCESS) {
				await update_changeset_config(path, (config) => {
					const updated = {...config};
					updated.access = access;
					updated.changelog = changelog;
					return updated;
				});
			}

			await spawn('git', ['add', dir]);

			if (dep) {
				await spawn(config.pm_cli, ['install', '-D', changelog]);
			}
		}

		// TODO small problem here where generated files don't get committed
		await invoke_task('sync', {install: inited || !dep}); // after installing above, and in all cases

		if (message) {
			// TODO see the helper below, simplify this to CLI flags when support is added to Changesets
			const changeset_adder = await create_changeset_adder(package_json.name, dir, message, bump);
			await spawn_cli(found_changeset_cli, ['add', '--empty'], log);
			await changeset_adder();
			if (!(await git_check_fully_staged_workspace())) {
				await spawn('git', ['commit', '-m', message]);
				await git_push_to_create(origin);
			}
		} else {
			await spawn_cli(found_changeset_cli, [], log);
			await spawn('git', ['add', dir]);
		}
	},
};

/**
 * TODO ideally this wouldn't exist and we'd use CLI flags, but they doesn't exist yet
 * @see https://github.com/changesets/changesets/pull/1121
 */
const create_changeset_adder = async (
	repo_name: string,
	dir: string,
	message: string,
	bump: ChangesetBump,
): Promise<() => Promise<void>> => {
	const filenames_before = await readdir(dir);
	return async () => {
		const filenames_after = await readdir(dir);
		const filenames_added = filenames_after.filter((p) => !filenames_before.includes(p));
		if (!filenames_added.length) {
			throw Error('expected to find a new changeset file');
		}
		if (filenames_added.length !== 1) {
			throw Error('expected to find exactly one new changeset file');
		}
		const path = join(dir, filenames_added[0]!);
		const contents = create_new_changeset(repo_name, message, bump);
		await writeFile(path, contents, 'utf8');
		await spawn('git', ['add', path]);
	};
};

const create_new_changeset = (
	repo_name: string,
	message: string,
	bump: ChangesetBump,
): string => `---
"${repo_name}": ${bump}
---

${message}
`;

type ChangesetCallback = (config: WrittenConfig) => WrittenConfig | Promise<WrittenConfig>;

type UpdateWrittenConfig = (path: string, cb: ChangesetCallback) => Promise<boolean>;

// TODO refactor all of this with zod and package_json helpers - util file helper? JSON parse pluggable

const update_changeset_config: UpdateWrittenConfig = async (path, cb) => {
	const config_contents = await load_changeset_config_contents(path);
	const config = parse_changeset_config(config_contents);

	const updated = await cb(config);

	const serialized = serialize_changeset_config(updated);

	if (serialized === config_contents) {
		return false;
	}

	await write_changeset_config(path, serialized);
	return true;
};

const load_changeset_config_contents = (path: string): Promise<string> => readFile(path, 'utf8');

const write_changeset_config = (path: string, serialized: string): Promise<void> =>
	writeFile(path, serialized);

const serialize_changeset_config = (config: WrittenConfig): string =>
	JSON.stringify(config, null, '\t') + '\n';

const parse_changeset_config = (contents: string): WrittenConfig => JSON.parse(contents);
