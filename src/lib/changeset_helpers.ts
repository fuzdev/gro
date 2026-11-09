import * as s from '@fuzdev/fuz_schema/schema.ts';

import type { FsDir } from './fs.ts';
import { is_fs_error } from './fs_error.ts';

export const CHANGESET_RESTRICTED_ACCESS = 'restricted';
export const CHANGESET_PUBLIC_ACCESS = 'public';

export const ChangesetAccess = s.one_of([CHANGESET_RESTRICTED_ACCESS, CHANGESET_PUBLIC_ACCESS]);
export type ChangesetAccess = s.Infer<typeof ChangesetAccess>;

export const CHANGESET_CLI = 'changeset';

export const CHANGESET_DIR = '.changeset';

export const ChangesetBump = s.one_of(['patch', 'minor', 'major']);
export type ChangesetBump = s.Infer<typeof ChangesetBump>;

/**
 * Whether `project` has a changeset to release, read the way Changesets does:
 * a markdown file in `.changeset` other than its `README.md`.
 * @param project - the project root
 */
export const changeset_has_pending = async (project: FsDir): Promise<boolean> => {
	try {
		for await (const { name, kind } of project.list(CHANGESET_DIR)) {
			if (kind === 'file' && name.endsWith('.md') && name !== 'README.md') return true;
		}
	} catch (error) {
		if (!is_fs_error(error, 'not_found')) throw error;
	}
	return false;
};
