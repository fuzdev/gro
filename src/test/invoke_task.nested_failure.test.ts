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
let infos: Array<Array<unknown>>;

beforeEach(() => {
	dir = mkdtempSync(join(tmpdir(), 'gro_invoke_task_nested_'));
	errors = [];
	infos = [];
});

afterEach(() => {
	rmSync(dir, { recursive: true, force: true });
});

/**
 * Writes an outer task whose `run` body is `body`, returning its path.
 */
const write_outer = (body: string): string => {
	const outer = join(dir, 'outer.task.ts');
	writeFileSync(outer, `export const task = {run: ({invoke_task}) => ${body}};\n`);
	return outer;
};

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
			{
				config: create_empty_gro_config(),
				watching: { graph: create_mock_module_graph(), watcher: create_mock_watcher() },
				// the tasks' loggers are its children, logging at `info` despite Vitest's default `off`
				parent_log: new Logger('test', {
					level: 'info',
					colors: false,
					console: {
						...console,
						log: (...args: Array<unknown>) => infos.push(args),
						error: (...args: Array<unknown>) => errors.push(args)
					} as unknown as Console
				})
			}
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

	test("a nested task's invalid args fail under the nested task, reported once", async () => {
		const error = await invoke_outer(write_outer(`invoke_task('gen', {bogus: 1})`));
		assert.instanceOf(error, SilentError);
		// the outer's `SilentError` carries the nested one, whose cause is the validation `TaskError`
		assert.instanceOf(error.cause, SilentError);
		assert.strictEqual((error.cause.cause as Error | undefined)?.constructor.name, 'TaskError');
		const info = infos.map((args) => format(...args)).join('\n');
		assert.match(info, /\[test:[^\]]*outer\.task\.ts:gen\] 🞩 gen/, info);
		const text = errors.map((args) => format(...args)).join('\n');
		assert.strictEqual(count(text, "Failed task args validation for task 'gen'"), 1, text);
		assert.strictEqual(count(text, 'bogus'), 1, text);
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
