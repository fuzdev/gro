import * as s from '@fuzdev/fuz_schema/schema.ts';

import type { Task } from '#lib/task.ts';

export const Args = s.union([s.object({ a: s.string }), s.object({ b: s.string })]);
export type Args = s.Infer<typeof Args>;

export const task: Task<Args> = {
	Args,
	run: () => {}
};
