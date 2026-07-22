import { z } from 'zod';

export const CHANGESET_RESTRICTED_ACCESS = 'restricted';
export const CHANGESET_PUBLIC_ACCESS = 'public';

export const ChangesetAccess = z.enum([CHANGESET_RESTRICTED_ACCESS, CHANGESET_PUBLIC_ACCESS]);
export type ChangesetAccess = z.infer<typeof ChangesetAccess>;

export const CHANGESET_CLI = 'changeset';

export const CHANGESET_DIR = '.changeset';

export const ChangesetBump = z.enum(['patch', 'minor', 'major']);
export type ChangesetBump = z.infer<typeof ChangesetBump>;
