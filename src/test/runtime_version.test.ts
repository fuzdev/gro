import { test, assert } from 'vitest';

import { runtime_version } from '#lib/runtime_version.ts';

test('`runtime_version` is the runtime and its version', () => {
	assert.strictEqual(runtime_version, `node/${process.version}`);
});
