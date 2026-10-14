import * as s from '@fuzdev/fuz_schema/schema.ts';

import type { Task } from '#lib/task.ts';

export const Args = s.object({ name: s.string, loud: s.with_default(s.boolean, false) });
export type Args = s.Infer<typeof Args>;

export const task: Task<Args> = {
	Args,
	run: () => {}
};
