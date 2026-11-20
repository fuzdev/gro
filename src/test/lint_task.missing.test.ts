import { test, assert, vi } from 'vitest';

import { run_lint } from '#lib/lint.task.ts';

import { create_mock_logger } from './test_helpers.ts';

test('without ESLint, `lint` warns that nothing was linted and passes', async () => {
	const log = create_mock_logger();
	const find = vi.fn((_role: string) => null);
	await run_lint({ _: [] }, { raw_rest_args: [] }, { programs: { find }, log });
	assert.deepEqual(find.mock.calls, [['eslint_cli']]);
	const warnings = vi.mocked(log.warn).mock.calls.flat().join('\n');
	assert.include(warnings, 'ESLint is not installed, so nothing was linted');
	assert.strictEqual(vi.mocked(log.info).mock.calls.length, 0);
});
