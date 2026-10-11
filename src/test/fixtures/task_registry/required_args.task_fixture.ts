import { z } from 'zod';

import type { Task } from '#lib/task.ts';

export const Args = z.strictObject({ name: z.string(), loud: z.boolean().default(false) });
export type Args = z.infer<typeof Args>;

export const task: Task<Args> = {
	Args,
	run: () => {}
};
