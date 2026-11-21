/**
 * Whether the project's npm install matches its `package.json` and `package-lock.json`,
 * read from the files npm writes, with no npm process: `package.json`'s dependency specs
 * against the lockfile's root entry, the lockfile against npm's hidden lockfile
 * `node_modules/.package-lock.json`, and each installed package's version against the lockfile.
 *
 * It's a review signal against mistakes, like pulling without installing,
 * so a check that ran against another install than the locked one says so and names `npm i`.
 * It's no boundary: anything that can write `node_modules` can make it pass,
 * and installing is outside what Gro confines.
 *
 * @module
 */

import { to_error_message } from '@fuzdev/fuz_util/error.ts';

import {
	LOCKFILE_FILENAME,
	NODE_MODULES_DIRNAME,
	NPM_HIDDEN_LOCKFILE_PATH,
	PACKAGE_JSON_FILENAME
} from './constants.ts';
import type { FsDir } from './fs.ts';
import type { FsPaths } from './fs_paths.ts';
import { is_fs_error } from './fs_error.ts';
import { TaskError } from './task_error.ts';

/**
 * How the install differs from what `package.json` and the lockfile say:
 * - `spec` - a `package.json` dependency spec differs from the lockfile's root entry
 * - `no_hidden_lockfile` - npm's hidden lockfile is missing while a package is locked to install,
 *   so nothing records the install
 * - `unrecorded` - a locked package isn't in the hidden lockfile
 * - `differs` - a locked package's `version`, `resolved`, `integrity`, or `link`
 *   differs from the hidden lockfile's
 * - `extraneous` - the hidden lockfile records a package the lockfile doesn't lock
 * - `not_installed` - a locked package has no installed `package.json`
 * - `version` - an installed package's version differs from the locked one, or it has none
 */
export type NpmInstallMismatchKind =
	| 'spec'
	| 'no_hidden_lockfile'
	| 'unrecorded'
	| 'differs'
	| 'extraneous'
	| 'not_installed'
	| 'version';

/**
 * One way the install differs, with the package or field it's about.
 */
export interface NpmInstallMismatch {
	kind: NpmInstallMismatchKind;
	/**
	 * The lockfile's key for the package, like `node_modules/@scope/name`,
	 * or for `spec` the field and name, like `devDependencies.vitest`.
	 */
	key: string;
	message: string;
}

/**
 * What `npm_install_check` found: the install unchecked, with why,
 * or checked, with each mismatch and the linked packages it skipped.
 */
export type NpmInstallCheck =
	| { checked: false; reason: string }
	| {
			checked: true;
			mismatches: Array<NpmInstallMismatch>;
			/**
			 * The lockfile's keys of the installed packages that are symlinks, like an `npm link`
			 * or a `file:` dependency, whose versions follow their source, not the lockfile.
			 */
			linked: Array<string>;
	  };

// the `package.json` fields npm records in the lockfile's root entry
const DEPENDENCY_FIELDS = [
	'dependencies',
	'devDependencies',
	'optionalDependencies',
	'peerDependencies'
] as const;

// the fields of a locked package the hidden lockfile repeats
const LOCKED_FIELDS = ['version', 'resolved', 'integrity', 'link'] as const;

interface LockEntry {
	version?: unknown;
	resolved?: unknown;
	integrity?: unknown;
	link?: unknown;
	optional?: unknown;
	devOptional?: unknown;
	os?: unknown;
	cpu?: unknown;
	[key: string]: unknown;
}

/**
 * Checks the project's npm install against its `package.json` and `package-lock.json`,
 * reading only those, the hidden lockfile, and the installed packages' `package.json` files.
 * A locked package npm may leave uninstalled, an optional one or one limited to some platforms
 * by `os` or `cpu`, isn't missing when it's absent. A package installed as a symlink is skipped,
 * as is one beneath a scope directory linked from outside the project.
 * @param fs_paths - reads the project's files
 * @returns `checked: false` with no `package-lock.json`, a lockfile with no `packages`
 * (`lockfileVersion` 1), or a `node_modules` that's a symlink, not followed,
 * else the mismatches, empty when the install matches
 * @throws `TaskError` - when `package.json`, the lockfile, or the hidden lockfile isn't valid JSON
 * @throws `FsError` - when reading one fails other than by its absence
 */
export const npm_install_check = async (fs_paths: FsPaths): Promise<NpmInstallCheck> => {
	const { project } = fs_paths;
	const lockfile = await read_json(project, LOCKFILE_FILENAME);
	if (lockfile === null) {
		return { checked: false, reason: `the project has no ${LOCKFILE_FILENAME}` };
	}
	const locked = to_record(lockfile.packages);
	if (locked === null) {
		return {
			checked: false,
			reason: `${LOCKFILE_FILENAME} has no \`packages\`, which lockfileVersion 2 and later have`
		};
	}
	// a linked `node_modules`, like a worktree sharing a checkout's install, isn't this project's
	// own install, and the link isn't followed, wherever it points
	if ((await project.stat(NODE_MODULES_DIRNAME, { follow: false }))?.kind === 'symlink') {
		return { checked: false, reason: `${NODE_MODULES_DIRNAME} is a symlink` };
	}
	const mismatches: Array<NpmInstallMismatch> = [];

	// `package.json`'s specs against the lockfile's root entry, as `npm ci` compares them
	const package_json = (await read_json(project, PACKAGE_JSON_FILENAME)) ?? {};
	const root = to_record(locked['']) ?? {};
	for (const field of DEPENDENCY_FIELDS) {
		const specified = to_record(package_json[field]) ?? {};
		const recorded = to_record(root[field]) ?? {};
		for (const name of new Set([...Object.keys(specified), ...Object.keys(recorded)])) {
			const a = specified[name];
			const b = recorded[name];
			if (a === b) continue;
			mismatches.push({
				kind: 'spec',
				key: `${field}.${name}`,
				message:
					`${PACKAGE_JSON_FILENAME}'s ${field} has ${name} ${to_shown(a)}, ` +
					`${LOCKFILE_FILENAME}'s ${to_shown(b)}`
			});
		}
	}

	// the installed packages, each a key under `node_modules`, the links skipped with what's in them
	const keys = Object.keys(locked).filter(is_installed_key);
	const linked = (
		await Promise.all(
			keys.map(async (key) => {
				try {
					return (await project.stat(key, { follow: false }))?.kind === 'symlink' ? key : null;
				} catch (error) {
					// beneath a scope directory linked from outside the project, linked as it is
					if (is_fs_error(error, 'outside_root')) return key;
					throw error;
				}
			})
		)
	).filter((key) => key !== null);
	const is_skipped = (key: string): boolean =>
		linked.some((link) => key === link || key.startsWith(link + '/'));

	// the lockfile against npm's record of what it installed
	const hidden_lockfile = await read_json(project, NPM_HIDDEN_LOCKFILE_PATH);
	const hidden = hidden_lockfile === null ? null : (to_record(hidden_lockfile.packages) ?? {});
	// npm writes no `node_modules`, so no hidden lockfile, when it has nothing to install
	if (hidden === null && keys.some((key) => !may_be_absent(to_lock_entry(locked[key])))) {
		mismatches.push({
			kind: 'no_hidden_lockfile',
			key: NPM_HIDDEN_LOCKFILE_PATH,
			message: `${NPM_HIDDEN_LOCKFILE_PATH} is missing, so nothing records the install`
		});
	} else if (hidden !== null) {
		for (const key of keys) {
			if (is_skipped(key)) continue;
			const entry = to_lock_entry(locked[key]);
			const installed = to_record(hidden[key]);
			if (installed === null) {
				if (!may_be_absent(entry)) {
					mismatches.push({
						kind: 'unrecorded',
						key,
						message: `${key} is locked, not recorded as installed`
					});
				}
				continue;
			}
			for (const field of LOCKED_FIELDS) {
				if (entry[field] === installed[field]) continue;
				mismatches.push({
					kind: 'differs',
					key,
					message:
						`${key}'s ${field} is ${to_shown(entry[field])} locked, ` +
						`${to_shown(installed[field])} installed`
				});
			}
		}
		for (const key of Object.keys(hidden)) {
			if (!is_installed_key(key) || Object.hasOwn(locked, key) || is_skipped(key)) continue;
			mismatches.push({ kind: 'extraneous', key, message: `${key} is installed, not locked` });
		}
	}

	// each installed package's own version, which the hidden lockfile doesn't see change
	const installed_versions = await Promise.all(
		keys.map(async (key) => {
			const entry = to_lock_entry(locked[key]);
			if (is_skipped(key) || entry.link === true) return null;
			const installed = await read_json(project, key + '/' + PACKAGE_JSON_FILENAME);
			return { key, entry, installed };
		})
	);
	for (const found of installed_versions) {
		if (found === null) continue;
		const { key, entry, installed } = found;
		if (installed === null) {
			if (!may_be_absent(entry)) {
				mismatches.push({ kind: 'not_installed', key, message: `${key} has no package.json` });
			}
			continue;
		}
		const { version } = installed;
		// a package the lockfile locks no version of, like a versionless git dependency
		if (version === entry.version) continue;
		if (version === undefined) {
			mismatches.push({
				kind: 'version',
				key,
				message: `${key}'s package.json has no version, ${to_shown(entry.version)} locked`
			});
		} else if (version !== entry.version) {
			mismatches.push({
				kind: 'version',
				key,
				message: `${key} is ${to_shown(version)} installed, ${to_shown(entry.version)} locked`
			});
		}
	}

	return { checked: true, mismatches, linked };
};

// a lockfile key for an installed package, `node_modules/a` or a nested `node_modules/a/node_modules/b`,
// not the root `''` or a link's target outside `node_modules`
const is_installed_key = (key: string): boolean =>
	key.startsWith(NODE_MODULES_DIRNAME + '/') && key !== NPM_HIDDEN_LOCKFILE_PATH;

// npm leaves an optional package uninstalled when it fails, and one limited by `os` or `cpu`
// on another platform, which a check reading only files can't tell from this one
const may_be_absent = (entry: LockEntry): boolean =>
	entry.optional === true ||
	entry.devOptional === true ||
	entry.os !== undefined ||
	entry.cpu !== undefined;

const to_record = (value: unknown): Record<string, unknown> | null =>
	value !== null && typeof value === 'object' && !Array.isArray(value)
		? (value as Record<string, unknown>)
		: null;

const to_lock_entry = (value: unknown): LockEntry => to_record(value) ?? {};

const to_shown = (value: unknown): string => (value === undefined ? 'none' : JSON.stringify(value));

// the JSON object at `path` in the project, `null` when nothing's there
const read_json = async (project: FsDir, path: string): Promise<Record<string, unknown> | null> => {
	const contents = await project.read_text_if_exists(path);
	if (contents === null) return null;
	let parsed: unknown;
	try {
		parsed = JSON.parse(contents);
	} catch (error) {
		throw new TaskError(`${path} isn't valid JSON: ${to_error_message(error)}`, { cause: error });
	}
	return to_record(parsed) ?? {};
};
