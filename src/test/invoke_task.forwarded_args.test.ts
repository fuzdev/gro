import { describe, test, expect, vi, afterEach } from 'vitest';
import { args_parse } from '@fuzdev/fuz_util/args.ts';
import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';

import { invoke_task } from '#lib/invoke_task.ts';
import { run_task } from '#lib/run_task.ts';
import type { GroConfig } from '#lib/gro_config.ts';
import type { Filer } from '#lib/filer.ts';

vi.mock('#lib/run_task.ts', () => ({
	run_task: vi.fn(async () => ({ ok: true }))
}));

vi.mock('#lib/task.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/task.ts')>()),
	find_tasks: vi.fn(async () => ({
		ok: true,
		value: { resolved_input_files: [{ resolved_input_path: { is_directory: false } }] }
	})),
	load_tasks: vi.fn(async () => ({
		ok: true,
		value: { modules: [{ name: 'x', mod: { task: {} } }] }
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
		process.argv = ['node', 'gro', 'x', '--color', '-C', '--', 'gro', 'x', '--color'];
		await invoke_task(
			'x',
			{ color: true, C: true },
			{ task_root_dirs: [] } as unknown as GroConfig,
			{ inited: false } as Filer,
			null
		);
		expect(vi.mocked(run_task)).toHaveBeenCalledTimes(1);
		const parsed = args_parse(vi.mocked(run_task).mock.calls[0]![1], ColorArgs);
		expect(parsed.ok && parsed.value).toEqual({ color: true, 'no-color': false });
	});
});
