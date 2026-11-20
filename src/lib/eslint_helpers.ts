import type { FsPaths } from './fs_paths.ts';

/**
 * The config files ESLint reads at a project's root, in the order ESLint looks for them:
 * the flat config names ESLint 9 finds, then the legacy `.eslintrc` names
 * an older ESLint, or ESLint 9 with `ESLINT_USE_FLAT_CONFIG=false`, reads.
 * One of them means the project means to lint.
 */
export const ESLINT_CONFIG_FILENAMES: ReadonlyArray<string> = [
	'eslint.config.js',
	'eslint.config.mjs',
	'eslint.config.cjs',
	'eslint.config.ts',
	'eslint.config.mts',
	'eslint.config.cts',
	'.eslintrc.js',
	'.eslintrc.cjs',
	'.eslintrc.yaml',
	'.eslintrc.yml',
	'.eslintrc.json',
	'.eslintrc'
];

/**
 * Finds the first ESLint config file at the project's root.
 * Any entry at a name counts, a symlink unfollowed, as it shows the project means to lint
 * wherever it points, even nowhere.
 * Only the root is read, though ESLint itself also looks in the directories above it.
 * @param fs_paths - reads the project's root
 * @returns the config's file name, or `null` when the project has none
 * @throws `FsError` - when checking a name fails other than by its absence
 */
export const find_eslint_config = async (fs_paths: FsPaths): Promise<string | null> => {
	for (const name of ESLINT_CONFIG_FILENAMES) {
		if ((await fs_paths.project.stat(name, { follow: false })) !== null) return name;
	}
	return null;
};
