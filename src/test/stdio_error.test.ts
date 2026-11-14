import { describe, test, assert } from 'vitest';

import { EnvError, is_env_error } from '#lib/env_error.ts';
import { StdioError, is_stdio_error } from '#lib/stdio_error.ts';

describe('StdioError', () => {
	test('is typed and frozen, and matches across copies of the class', () => {
		const error = new StdioError('io_error', 'stdout');
		assert.strictEqual(error.kind, 'io_error');
		assert.strictEqual(error.target, 'stdout');
		assert.ok(Object.isFrozen(StdioError));
		assert.ok(Object.isFrozen(StdioError.prototype));
		assert.ok(is_stdio_error(error, 'io_error'));
		class Other extends Error {
			override name = 'StdioError';
			kind = 'io_error';
		}
		Object.defineProperty(Other, 'name', { value: 'StdioError' });
		assert.ok(is_stdio_error(new Other(), 'io_error'));
		assert.ok(!is_stdio_error(new Error('x')));
	});

	test('is not an `EnvError`, the two families apart', () => {
		assert.ok(!is_env_error(new StdioError('io_error', 'stdin')));
		assert.ok(!is_stdio_error(new EnvError('not_granted', 'A_NAME')));
	});
});
