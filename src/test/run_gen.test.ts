import { test, expect, assert } from 'vitest';
import { resolve } from 'node:path';

import { spawn_with_loader } from '#lib/gro_helpers.ts';
import { spawn } from '#lib/spawn.ts';
import { run_gen } from '#lib/run_gen.ts';
import type { GenfileModuleMeta } from '#lib/gen.ts';

import {
	LOADER_SOURCE_PATH,
	TEST_TIMEOUT_MD,
	create_mock_config,
	create_mock_logger,
	create_mock_timings
} from './test_helpers.ts';

test(
	'basic behavior',
	async () => {
		const test_script = resolve('src/test/fixtures/test_run_gen.ts');

		const node = spawn.program('node', { pass_env: 'all' });
		const result = await spawn_with_loader(node, LOADER_SOURCE_PATH, test_script, [], undefined);

		// The spawn_with_loader function handles output automatically
		expect(result).toEqual({ kind: 'exited', code: 0 });
	},
	TEST_TIMEOUT_MD
);

const origin_id = resolve('src/foo.gen.ts');

const run = async (
	gen: GenfileModuleMeta['mod']['gen'],
	format_file?: (content: string) => string
) =>
	run_gen(
		[{ id: origin_id, mod: { gen } }],
		await create_mock_config(),
		create_mock_logger(),
		create_mock_timings(),
		() => Promise.resolve(),
		format_file
	);

test('a genfile returning content that is not a string fails, naming it and what it returned', async () => {
	// a stale `export const gen = gen_task_registry;` calls the factory as the gen function
	const results = await run(() => ({ generate: () => '', inputs: {} }) as any);
	assert.strictEqual(results.successes.length, 0);
	assert.strictEqual(results.failures.length, 1);
	const [failure] = results.failures;
	assert.include(failure!.error.message, 'Genfile ./src/foo.gen.ts returned an object');
	assert.include(failure!.error.message, 'whose `content` is missing');
	assert.include(failure!.reason, 'Error generating');
});

test('a genfile whose output fails to format fails, so none of its outputs are written', async () => {
	const results = await run(
		() => [
			{ filename: 'a.ts', content: 'ok' },
			{ filename: 'b.ts', content: 'bad' }
		],
		(content) => {
			if (content === 'bad') throw Error('unformattable');
			return content + '\n';
		}
	);
	// `write_gen_results` writes only the successes' files
	assert.strictEqual(results.successes.length, 0);
	assert.strictEqual(results.failures.length, 1);
	assert.strictEqual(results.failures[0]!.error.message, 'unformattable');
	assert.include(results.failures[0]!.reason, 'Error formatting');
	assert.include(results.failures[0]!.reason, 'b.ts');
});

test('a genfile whose outputs format succeeds with the formatted content', async () => {
	const results = await run(
		() => ({ content: 'ok' }),
		(content) => content + '\n'
	);
	assert.strictEqual(results.successes.length, 1);
	assert.deepEqual(
		results.successes[0]!.files.map((f) => f.content),
		['ok\n']
	);
});
