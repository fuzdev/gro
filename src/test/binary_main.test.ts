import { test, assert, vi, afterEach } from 'vitest';
import { Logger } from '@fuzdev/fuz_util/log.ts';
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

import { binary_main } from '#lib/binary_main.ts';
import { fs } from '#lib/fs.ts';
import { TaskError, is_task_error } from '#lib/task_error.ts';
import type { TaskContext } from '#lib/task.ts';
import { create_throwing_builtin_host_root } from './test_helpers.ts';

afterEach(() => {
	vi.restoreAllMocks();
});

interface BinaryRun {
	exit_codes: Array<number>;
	urls: Array<string>;
}

/**
 * Runs `binary_main` in a fresh project as the binary's entry would, the host's handles
 * throwing on use, recording the exit codes it sets and the URLs it imports.
 * @param project - the project's files by path, written before the run
 * @param import_url - stands in for the entry's `import()`
 * @param dir_name - the project directory's name, beneath a temp directory
 */
const run = async (
	args: Array<string>,
	project: Record<string, string>,
	import_url: (url: string) => Promise<Record<string, any>>,
	dir_name = 'project'
): Promise<BinaryRun & { dir: string }> => {
	const temp = realpathSync(mkdtempSync(join(tmpdir(), 'binary_main_')));
	const dir = join(temp, dir_name);
	const recorded: BinaryRun = { exit_codes: [], urls: [] };
	try {
		mkdirSync(join(dir, 'src'), { recursive: true });
		writeFileSync(join(dir, 'package.json'), '{"name": "x"}');
		for (const [path, contents] of Object.entries(project)) {
			writeFileSync(join(dir, path), contents);
		}
		const host_root = create_throwing_builtin_host_root();
		await binary_main({
			args,
			fs: { ...fs, project: await fs.open_dir(dir) },
			env: host_root.env,
			spawn: host_root.spawn,
			stdout: host_root.stdout,
			stderr: host_root.stderr,
			cwd: () => dir,
			set_exit_code: (code) => {
				recorded.exit_codes.push(code);
			},
			import_url: (url) => {
				recorded.urls.push(url);
				return import_url(url);
			},
			home: undefined,
			version: '1.2.3-binary',
			cli_name: 'xgro'
		});
	} finally {
		rmSync(temp, { recursive: true });
	}
	return { ...recorded, dir };
};

const silence_logs = () => ({
	error: vi.spyOn(Logger.prototype, 'error').mockImplementation(() => undefined),
	info: vi.spyOn(Logger.prototype, 'info').mockImplementation(() => undefined)
});

test('a failure is reported with no stack and sets the exit code from it, never thrown', async () => {
	const { error } = silence_logs();
	const { exit_codes } = await run(['--version'], { 'gro.config.ts': '' }, () =>
		Promise.reject(new TaskError('the config failed', { exit_code: 7 }))
	);
	assert.deepEqual(exit_codes, [7]);
	assert.strictEqual(error.mock.calls.length, 1);
	assert.include(String(error.mock.calls[0]![0]), 'the config failed');
	assert.notInclude(String(error.mock.calls[0]![0]), 'at ');
});

test('a run that succeeds sets no exit code, and names the binary as the running Gro', async () => {
	const { info } = silence_logs();
	const { exit_codes } = await run(['--version'], {}, () => Promise.reject(Error('no import')));
	assert.deepEqual(exit_codes, []);
	assert.ok(info.mock.calls.some((call) => String(call[0]).includes('1.2.3-binary')));
});

test("the listing names the binary's name in its usage", async () => {
	const { info } = silence_logs();
	const { exit_codes } = await run([], {}, () => Promise.reject(Error('no import')));
	assert.deepEqual(exit_codes, []);
	const listing = info.mock.calls.map((call) => String(call[0])).join('\n');
	assert.include(listing, 'xgro [name]');
	assert.notInclude(listing, ' gro [name]');
});

test("the project loader imports a module by its file URL, a path's `#`, `?`, and `%` escaped", async () => {
	silence_logs();
	const { exit_codes, urls, dir } = await run(
		['--version'],
		{ 'gro.config.ts': '' },
		() => Promise.resolve({ default: (config: unknown) => config }),
		'a#b?c%d e'
	);
	assert.deepEqual(exit_codes, []);
	const expected = pathToFileURL(join(dir, 'gro.config.ts')).href;
	assert.include(expected, '/a%23b%3Fc%25d%20e/');
	assert.deepEqual(urls, [expected]);
});

test('the module graph and the Svelte config refuse, naming the command and its Node run', async () => {
	silence_logs();
	const failures: Array<unknown> = [];
	const contexts: Array<TaskContext> = [];
	const settle = async (promise: Promise<unknown>): Promise<void> => {
		try {
			await promise;
			failures.push('resolved');
		} catch (error) {
			failures.push(error);
		}
	};
	const task_module = {
		task: {
			summary: 'reads what the binary has none of',
			run: async (ctx: TaskContext) => {
				contexts.push(ctx);
				await settle(ctx.graph.modules());
				await settle(ctx.graph.dependents('/x.ts'));
				await settle(ctx.graph.watch(() => undefined));
				await settle(ctx.svelte_config);
			}
		}
	};
	const { exit_codes } = await run(['task', 'src/x.task.ts'], { 'src/x.task.ts': '' }, (url) =>
		url.endsWith('/src/x.task.ts')
			? Promise.resolve(task_module)
			: Promise.reject(Error('unexpected import: ' + url))
	);
	assert.deepEqual(exit_codes, []);
	assert.strictEqual(contexts.length, 1);
	assert.deepEqual(contexts[0]!.gro, { version: '1.2.3-binary', dir: null });
	assert.strictEqual(failures.length, 4);
	for (const [i, failure] of failures.entries()) {
		assert.ok(is_task_error(failure), `failure ${i} is a task error`);
		const what = i < 3 ? 'the module graph' : 'the Svelte config';
		assert.strictEqual(
			failure.message,
			`\`gro task\` needs ${what}, which only the Node gro has: run \`node_modules/.bin/gro task\``
		);
	}
});
