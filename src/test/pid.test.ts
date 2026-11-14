import { test, assert } from 'vitest';

import { pid } from '#lib/pid.ts';

test('`pid` is the process id', () => {
	assert.strictEqual(pid, process.pid);
});
