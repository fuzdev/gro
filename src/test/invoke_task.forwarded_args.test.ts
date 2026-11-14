import { describe, test, expect, vi, afterEach } from 'vitest';
import { args_parse } from '@fuzdev/fuz_util/args.ts';
import { resolve } from 'node:path';
import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';

import { invoke_task } from '#lib/invoke_task.ts';
import { run_task } from '#lib/run_task.ts';
import type { GroConfig } from '#lib/gro_config.ts';
import { log_error_reasons } from '#lib/task_logging.ts';
import { SilentError } from '#lib/task.ts';
import {
	to_removed_builtin_prefix_message,
	to_task_path_without_verb_message
} from '#lib/internal/task_hints.ts';
import { create_mock_module_graph } from './test_helpers.ts';
import { create_project_fs_paths } from './fs_test_helpers.ts';
import { to_raw_rest_args } from '#lib/task_args.ts';

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

// the task paths resolve to Gro's own task files
const fs_paths = await create_project_fs_paths(process.cwd());

afterEach(() => {
	vi.clearAllMocks();
});

describe('invoke_task forwarded args', () => {
	const ColorArgs = s.object({
		color: meta(s.with_default(s.boolean, true), { aliases: ['c'] }),
		'no-color': meta(s.with_default(s.boolean, false), { aliases: ['C'] })
	});

	test('a forwarded flag wins a no-x pair against a later direct alias', async () => {
		const raw_rest_args = to_raw_rest_args([
			'check',
			'--color',
			'-C',
			'--',
			'gro',
			'check',
			'--color'
		]);
		await invoke_task(
			'check',
			{ color: true, C: true },
			{
				config: {} as unknown as GroConfig,
				fs_paths,
				raw_rest_args,
				graph: create_mock_module_graph()
			}
		);
		expect(vi.mocked(run_task)).toHaveBeenCalledTimes(1);
		const parsed = args_parse(vi.mocked(run_task).mock.calls[0]![1], ColorArgs);
		expect(parsed.ok && parsed.value).toEqual({ color: true, 'no-color': false });
	});

	test('a task path is forwarded args by `gro task` and its normalized path', async () => {
		const raw_rest_args = to_raw_rest_args([
			'check',
			'--',
			'gro',
			'task',
			'./src/lib/check.task.ts',
			'--a'
		]);
		await invoke_task(
			'src/lib/check.task.ts',
			{ b: true },
			{
				config: {} as unknown as GroConfig,
				fs_paths,
				raw_rest_args,
				graph: create_mock_module_graph()
			}
		);
		expect(vi.mocked(run_task)).toHaveBeenCalledTimes(1);
		const [task_meta, args] = vi.mocked(run_task).mock.calls[0]!;
		expect(task_meta.name).toBe('src/lib/check.task.ts');
		expect(args).toEqual({ b: true, a: true });
	});

	test('an absolute forwarded task path normalizes against the project root', async () => {
		const raw_rest_args = to_raw_rest_args([
			'check',
			'--',
			'gro',
			'task',
			resolve('src/lib/check.task.ts'),
			'--a'
		]);
		await invoke_task(
			'src/lib/check.task.ts',
			{ b: true },
			{
				config: {} as unknown as GroConfig,
				fs_paths,
				raw_rest_args,
				graph: create_mock_module_graph()
			}
		);
		expect(vi.mocked(run_task).mock.calls[0]![1]).toEqual({ b: true, a: true });
	});

	test('a builtin is not forwarded the args of a project task path', async () => {
		const raw_rest_args = to_raw_rest_args([
			'x',
			'--',
			'gro',
			'task',
			'src/lib/check.task.ts',
			'--a'
		]);
		await invoke_task(
			'check',
			{ b: true },
			{
				config: {} as unknown as GroConfig,
				fs_paths,
				raw_rest_args,
				graph: create_mock_module_graph()
			}
		);
		expect(vi.mocked(run_task).mock.calls[0]![1]).toEqual({ b: true });
	});

	test('a forwarded path without `gro task` or with the removed prefix fails with the CLI hint', async () => {
		for (const input of ['src/lib/check.task.ts', 'gro/check']) {
			const raw_rest_args = to_raw_rest_args(['check', '--', 'gro', input, '--a']);
			await expect(
				invoke_task(
					'check',
					{},
					{
						config: {} as unknown as GroConfig,
						fs_paths,
						raw_rest_args,
						graph: create_mock_module_graph()
					}
				)
			).rejects.toBeInstanceOf(SilentError);
		}
		expect(vi.mocked(run_task)).not.toHaveBeenCalled();
		const reasons = vi.mocked(log_error_reasons).mock.calls.map((c) => c[1]);
		expect(reasons).toEqual([
			[to_task_path_without_verb_message('src/lib/check.task.ts', process.cwd(), false)],
			[to_removed_builtin_prefix_message('gro/check')]
		]);
	});

	test("a tool section is never parsed, so a malformed one doesn't fail a task", async () => {
		const raw_rest_args = to_raw_rest_args([
			'check',
			'--',
			'eslint',
			'--no-_',
			'--',
			'gro',
			'check',
			'--a'
		]);
		await invoke_task(
			'check',
			{},
			{
				config: {} as unknown as GroConfig,
				fs_paths,
				raw_rest_args,
				graph: create_mock_module_graph()
			}
		);
		expect(vi.mocked(run_task).mock.calls[0]![1]).toEqual({ a: true });
	});
});
