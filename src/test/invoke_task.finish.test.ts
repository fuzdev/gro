import { describe, test, assert, afterEach, beforeEach } from 'vitest';
import { Logger } from '@fuzdev/fuz_util/log.ts';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { format } from 'node:util';

import { invoke_task } from '#lib/invoke_task.ts';
import { create_empty_gro_config, type GroConfig } from '#lib/gro_config.ts';
import { SilentError, type TaskContext } from '#lib/task.ts';
import type { FsPaths } from '#lib/fs_paths.ts';
import { create_project_fs_paths } from './fs_test_helpers.ts';

let dir: string;
let config: GroConfig;
let fs_paths: FsPaths;
let lines: Array<string>;

beforeEach(async () => {
	dir = mkdtempSync(join(tmpdir(), 'gro_invoke_task_finish_'));
	fs_paths = await create_project_fs_paths(dir);
	// the owned graph watches only the empty temp dir, should it ever load
	config = { ...create_empty_gro_config(), watch: { dir, filter: null } };
	lines = [];
});

afterEach(() => {
	rmSync(dir, { recursive: true, force: true });
	delete (globalThis as any).gro_invoke_task_finish_ctx;
});

// the tasks' loggers are its children, logging at `debug` (where timings print) despite Vitest's default `off`
const create_log = (): Logger =>
	new Logger('test', {
		level: 'debug',
		colors: false,
		console: {
			...console,
			log: (...args: Array<unknown>) => lines.push(format(...args)),
			error: (...args: Array<unknown>) => lines.push(format(...args))
		} as unknown as Console
	});

describe('invoke_task finish', () => {
	test('only the top-level task prints the timings its nested tasks share', async () => {
		const inner = join(dir, 'inner.task.ts');
		const outer = join(dir, 'outer.task.ts');
		writeFileSync(inner, `export const task = {run: () => {}};\n`);
		writeFileSync(
			outer,
			`export const task = {run: ({invoke_task}) => invoke_task(${JSON.stringify(inner)})};\n`
		);

		await invoke_task(outer, {}, { config, fs_paths, parent_log: create_log() });

		const totals = lines.filter((line) => line.includes('🕒'));
		assert.strictEqual(totals.length, 1, lines.join('\n'));
		assert.notInclude(totals[0], 'inner.task.ts');
		// the shared timings include the nested task's
		assert.ok(lines.some((line) => line.includes('run task ') && line.includes('inner.task.ts')));
	});

	test('a failing task still closes the graph it owns', async () => {
		const failing = join(dir, 'failing.task.ts');
		writeFileSync(
			failing,
			`export const task = {run: (ctx) => {
	globalThis.gro_invoke_task_finish_ctx = ctx;
	throw new Error('boom');
}};\n`
		);

		let error;
		try {
			await invoke_task(failing, {}, { config, fs_paths, parent_log: create_log() });
		} catch (e) {
			error = e;
		}
		assert.instanceOf(error, SilentError);

		const { graph } = (globalThis as any).gro_invoke_task_finish_ctx as TaskContext;
		try {
			let graph_error;
			try {
				await graph.modules();
			} catch (e) {
				graph_error = e;
			}
			assert.instanceOf(graph_error, Error);
			assert.strictEqual(graph_error.message, 'ModuleGraph is closed');
		} finally {
			await graph.close(); // idempotent, cleans up on failure
		}
	});
});
