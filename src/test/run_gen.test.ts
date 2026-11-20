import { test, expect, assert, vi } from 'vitest';
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
	create_mock_svelte_config,
	create_mock_timings,
	paths
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
		{
			config: await create_mock_config(),
			log: create_mock_logger(),
			timings: create_mock_timings(),
			load_svelte_config: () => Promise.resolve(create_mock_svelte_config())
		},
		paths.root,
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

test('a genfile with invalid inputs fails, naming it, and runs nothing', async () => {
	let generated = false;
	const results = await run({
		generate: () => {
			generated = true;
			return 'content';
		},
		inputs: { patterns: ['./src/*.json'] }
	});
	const [failure] = results.failures;
	assert.ok(failure);
	assert.strictEqual(failure.id, origin_id);
	assert.include(failure.error.message, 'src/foo.gen.ts');
	assert.include(failure.error.message, 'write `/src/*.json`');
	assert.ok(!generated);
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

test("a genfile's `svelte_config` is the base's, loaded only when the genfile reads it, and it gets no `invoke_task`", async () => {
	const svelte_config = Promise.resolve(create_mock_svelte_config());
	const load_svelte_config = vi.fn(() => svelte_config);
	let read: unknown;
	let has_invoke: unknown;
	const results = await run_gen(
		[
			{ id: origin_id, mod: { gen: () => '' } },
			{
				id: resolve('src/bar.gen.ts'),
				mod: {
					gen: (ctx) => {
						read = ctx.svelte_config;
						has_invoke = 'invoke_task' in ctx;
						return '';
					}
				}
			}
		],
		{
			config: await create_mock_config(),
			log: create_mock_logger(),
			timings: create_mock_timings(),
			load_svelte_config
		},
		paths.root
	);
	assert.strictEqual(results.failures.length, 0);
	assert.strictEqual(read, svelte_config);
	assert.strictEqual(has_invoke, false);
	assert.strictEqual(load_svelte_config.mock.calls.length, 1);
});
