import * as s from '@fuzdev/fuz_schema/schema.ts';

export const CHANGESET_RESTRICTED_ACCESS = 'restricted';
export const CHANGESET_PUBLIC_ACCESS = 'public';

export const ChangesetAccess = s.one_of([CHANGESET_RESTRICTED_ACCESS, CHANGESET_PUBLIC_ACCESS]);
export type ChangesetAccess = s.Infer<typeof ChangesetAccess>;

export const CHANGESET_CLI = 'changeset';

export const CHANGESET_DIR = '.changeset';

export const ChangesetBump = s.one_of(['patch', 'minor', 'major']);
export type ChangesetBump = s.Infer<typeof ChangesetBump>;
