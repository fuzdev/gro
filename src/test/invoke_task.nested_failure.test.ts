import { describe, test, assert, afterEach, beforeEach } from 'vitest';
import { Logger } from '@fuzdev/fuz_util/log.ts';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { format } from 'node:util';

import { invoke_task } from '#lib/invoke_task.ts';
import { create_empty_gro_config } from '#lib/gro_config.ts';
import { SilentError } from '#lib/task.ts';
import { create_mock_module_graph, create_mock_watcher } from './test_helpers.ts';

let dir: string;
let errors: Array<Array<unknown>>;

beforeEach(() => {
	dir = mkdtempSync(join(tmpdir(), 'gro_invoke_task_nested_'));
	errors = [];
});

afterEach(() => {
	rmSync(dir, { recursive: true, force: true });
});

/**
 * Writes an outer task that invokes an inner task throwing `inner_throw`, returning the outer's path.
 */
const write_nested = (inner_throw: string): string => {
	const inner = join(dir, 'inner.task.ts');
	const outer = join(dir, 'outer.task.ts');
	writeFileSync(
		inner,
		`import {TaskError} from '${join(process.cwd(), 'src/lib/task.ts')}';
export const task = {run: () => {${inner_throw}}};\n`
	);
	writeFileSync(
		outer,
		`export const task = {run: ({invoke_task}) => invoke_task(${JSON.stringify(inner)})};\n`
	);
	return outer;
};

const invoke_outer = async (outer: string): Promise<unknown> => {
	try {
		await invoke_task(
			outer,
			{},
			create_empty_gro_config(),
			{ graph: create_mock_module_graph(), watcher: create_mock_watcher() },
			null,
			// the tasks' loggers are its children, logging at `info` despite Vitest's default `off`
			new Logger('test', {
				level: 'info',
				colors: false,
				console: {
					...console,
					log: () => {},
					error: (...args: Array<unknown>) => errors.push(args)
				} as unknown as Console
			})
		);
	} catch (error) {
		return error;
	}
	assert.fail('expected the invoke to fail');
};

const count = (text: string, part: string): number => text.split(part).length - 1;

describe('invoke_task nested failure', () => {
	test('a nested `TaskError` is reported once, the outer task says only it failed', async () => {
		const error = await invoke_outer(write_nested(`throw new TaskError('inner boom');`));
		assert.instanceOf(error, SilentError);
		const text = errors.map((args) => format(...args)).join('\n');
		assert.strictEqual(count(text, 'inner boom'), 1, text);
		assert.include(text, 'outer.task.ts');
		assert.include(text, 'failed.');
		assert.notInclude(text, 'Unexpected error');
	});

	test("a nested unexpected error's stack is logged once", async () => {
		const error = await invoke_outer(write_nested(`throw new Error('inner crash');`));
		assert.instanceOf(error, SilentError);
		const text = errors.map((args) => format(...args)).join('\n');
		assert.strictEqual(count(text, 'inner crash'), 1, text);
		assert.strictEqual(count(text, 'Unexpected error'), 1, text);
		assert.include(text, 'inner.task.ts:2'); // the stack
	});
});
