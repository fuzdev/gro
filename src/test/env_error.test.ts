import { describe, test, assert } from 'vitest';

import { EnvError, is_env_error } from '#lib/env_error.ts';

describe('EnvError', () => {
	test('is typed and frozen, and matches across copies of the class', () => {
		const error = new EnvError('not_granted', 'A_NAME');
		assert.strictEqual(error.kind, 'not_granted');
		assert.strictEqual(error.target, 'A_NAME');
		assert.ok(Object.isFrozen(EnvError));
		assert.ok(Object.isFrozen(EnvError.prototype));
		assert.ok(is_env_error(error, 'not_granted'));
		class Other extends Error {
			override name = 'EnvError';
			kind = 'other';
		}
		Object.defineProperty(Other, 'name', { value: 'EnvError' });
		assert.ok(is_env_error(new Other()));
		assert.ok(!is_env_error(new Other(), 'not_granted'));
		assert.ok(!is_env_error(new Error('x')));
	});
});
