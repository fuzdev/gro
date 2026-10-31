import { describe, test, expect, vi, afterEach } from 'vitest';
import { args_parse } from '@fuzdev/fuz_util/args.ts';
import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';

import { invoke_task } from '#lib/invoke_task.ts';
import { run_task } from '#lib/run_task.ts';
import type { GroConfig } from '#lib/gro_config.ts';
import { log_error_reasons } from '#lib/task_logging.ts';
import {
	SilentError,
	to_removed_builtin_prefix_message,
	to_task_path_without_verb_message
} from '#lib/task.ts';
import { create_mock_module_graph, create_mock_watcher } from './test_helpers.ts';

vi.mock('#lib/run_task.ts', () => ({
	run_task: vi.fn(async () => ({ ok: true }))
}));

vi.mock('#lib/task_logging.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/task_logging.ts')>()),
	log_error_reasons: vi.fn()
}));

vi.mock('#lib/task.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/task.ts')>()),
	load_task: vi.fn(async (id: string, name: string) => ({
		ok: true,
		value: { id, name, mod: { task: {} } }
	}))
}));

const original_argv = process.argv;

afterEach(() => {
	process.argv = original_argv;
	vi.clearAllMocks();
});

describe('invoke_task forwarded args', () => {
	const ColorArgs = s.object({
		color: meta(s.with_default(s.boolean, true), { aliases: ['c'] }),
		'no-color': meta(s.with_default(s.boolean, false), { aliases: ['C'] })
	});

	test('a forwarded flag wins a no-x pair against a later direct alias', async () => {
		process.argv = ['node', 'gro', 'check', '--color', '-C', '--', 'gro', 'check', '--color'];
		await invoke_task(
			'check',
			{ color: true, C: true },
			{
				config: {} as unknown as GroConfig,
				watching: { graph: create_mock_module_graph(), watcher: create_mock_watcher() }
			}
		);
		expect(vi.mocked(run_task)).toHaveBeenCalledTimes(1);
		const parsed = args_parse(vi.mocked(run_task).mock.calls[0]![1], ColorArgs);
		expect(parsed.ok && parsed.value).toEqual({ color: true, 'no-color': false });
	});

	test('a task path is forwarded args by `gro task` and its normalized path', async () => {
		process.argv = ['node', 'gro', 'check', '--', 'gro', 'task', './src/lib/check.task.ts', '--a'];
		await invoke_task(
			'src/lib/check.task.ts',
			{ b: true },
			{
				config: {} as unknown as GroConfig,
				watching: { graph: create_mock_module_graph(), watcher: create_mock_watcher() }
			}
		);
		expect(vi.mocked(run_task)).toHaveBeenCalledTimes(1);
		const [task_meta, args] = vi.mocked(run_task).mock.calls[0]!;
		expect(task_meta.name).toBe('src/lib/check.task.ts');
		expect(args).toEqual({ b: true, a: true });
	});

	test('a builtin is not forwarded the args of a project task path', async () => {
		process.argv = ['node', 'gro', 'x', '--', 'gro', 'task', 'src/lib/check.task.ts', '--a'];
		await invoke_task(
			'check',
			{ b: true },
			{
				config: {} as unknown as GroConfig,
				watching: { graph: create_mock_module_graph(), watcher: create_mock_watcher() }
			}
		);
		expect(vi.mocked(run_task).mock.calls[0]![1]).toEqual({ b: true });
	});

	test('a forwarded path without `gro task` or with the removed prefix fails with the CLI hint', async () => {
		for (const input of ['src/lib/check.task.ts', 'gro/check']) {
			process.argv = ['node', 'gro', 'check', '--', 'gro', input, '--a'];
			await expect(
				invoke_task(
					'check',
					{},
					{
						config: {} as unknown as GroConfig,
						watching: { graph: create_mock_module_graph(), watcher: create_mock_watcher() }
					}
				)
			).rejects.toBeInstanceOf(SilentError);
		}
		expect(vi.mocked(run_task)).not.toHaveBeenCalled();
		const reasons = vi.mocked(log_error_reasons).mock.calls.map((c) => c[1]);
		expect(reasons).toEqual([
			[to_task_path_without_verb_message('src/lib/check.task.ts')],
			[to_removed_builtin_prefix_message('gro/check')]
		]);
	});

	test("a tool section is never parsed, so a malformed one doesn't fail a task", async () => {
		process.argv = ['node', 'gro', 'check', '--', 'eslint', '--no-_', '--', 'gro', 'check', '--a'];
		await invoke_task(
			'check',
			{},
			{
				config: {} as unknown as GroConfig,
				watching: { graph: create_mock_module_graph(), watcher: create_mock_watcher() }
			}
		);
		expect(vi.mocked(run_task).mock.calls[0]![1]).toEqual({ a: true });
	});
});
