import { test, assert } from 'vitest';

import { assert_no_forwarded_info_flag } from '#lib/internal/forwarded_info_flags.ts';
import { is_task_error } from '#lib/task_error.ts';

const to_refusal = (tool: string, argv: Array<string>, version: boolean): string | null => {
	try {
		assert_no_forwarded_info_flag(tool, argv, version);
		return null;
	} catch (error) {
		assert(is_task_error(error));
		return error.message;
	}
};

test('the exact help and version forms are refused, naming the tool and running it directly', () => {
	assert.strictEqual(
		to_refusal('eslint', ['--cache', '--help'], true),
		'`--help` forwarded to eslint asks it for its help, not a run, ' +
			'so the step could pass without one - run `eslint --help` directly'
	);
	assert.strictEqual(
		to_refusal('tsc', ['-v'], true),
		'`-v` forwarded to tsc asks it for its version, not a run, ' +
			'so the step could pass without one - run `tsc -v` directly'
	);
	assert.include(to_refusal('svelte-check', ['-h'], true), '`-h` forwarded to svelte-check');
	assert.include(to_refusal('svelte-check', ['--version'], true), '`--version` forwarded to');
});

test('without `version`, only help is refused', () => {
	assert.strictEqual(to_refusal('vitest', ['--version', '-v'], false), null);
	assert.include(to_refusal('vitest', ['-v', '-h'], false), '`-h` forwarded to vitest');
});

test('only whole args in the exact forms, a guardrail against slips', () => {
	assert.strictEqual(
		to_refusal('eslint', ['--help=x', '-hv', '--no-help', '--helpful', '-V', '--Help'], true),
		null
	);
});
