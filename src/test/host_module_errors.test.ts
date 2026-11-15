import { describe, test, assert } from 'vitest';

import { EnvError } from '#lib/env_error.ts';
import { FsError } from '#lib/fs_error.ts';
import { NetError } from '#lib/net_error.ts';
import { SpawnError } from '#lib/spawn_error.ts';
import { StdioError } from '#lib/stdio_error.ts';

// each family's class, its target's property, and a kind it has
const FAMILIES = [
	{ Class: FsError, target: 'path', kind: 'io_error' },
	{ Class: SpawnError, target: 'program', kind: 'launch_failed' },
	{ Class: NetError, target: 'target', kind: 'not_granted' },
	{ Class: EnvError, target: 'target', kind: 'not_granted' },
	{ Class: StdioError, target: 'target', kind: 'io_error' }
] as const;

describe('the error classes of the host modules', () => {
	for (const { Class, target, kind } of FAMILIES) {
		describe(Class.name, () => {
			const Any = Class as unknown as new (...args: Array<unknown>) => Error & {
				kind: string;
			} & Record<string, unknown>;

			test('the message is exactly `kind: target`, and the detail is the cause', () => {
				const cause = new Error('what the OS said');
				const error = new Any(kind, 'the target', { cause });
				assert.strictEqual(error.message, `${kind}: the target`);
				assert.strictEqual(error.kind, kind);
				assert.strictEqual(error[target], 'the target');
				assert.strictEqual(error.cause, cause);
				assert.ok(!('cause' in new Any(kind, 'the target')));
			});

			test('takes no message: a third argument is the options', () => {
				assert.strictEqual(Class.length, 3);
				const error = new Any(kind, 'the target', 'a message');
				assert.strictEqual(error.message, `${kind}: the target`);
				assert.ok(!('cause' in error));
			});
		});
	}
});
