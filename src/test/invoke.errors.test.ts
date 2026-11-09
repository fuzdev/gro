import { describe, test, assert, onTestFinished } from 'vitest';
import { spawn, spawnSync, type StdioOptions } from 'node:child_process';
import { closeSync, existsSync, openSync } from 'node:fs';
import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';

import { spawn_with_loader_args } from '#lib/gro_helpers.ts';
import { SVELTEKIT_DEV_DIRNAME, SVELTEKIT_TSCONFIG_PATH } from '#lib/constants.ts';

import { LOADER_SOURCE_PATH, TEST_TIMEOUT_MD } from './test_helpers.ts';

const INVOKE_SOURCE_PATH = resolve('src/lib/invoke.ts');
const SPAWN_SOURCE_PATH = resolve('src/lib/spawn.ts');

test(
	'a TaskError raised before any task prints its message with no stack and exits 1',
	async () => {
		const dir = await mkdtemp(join(tmpdir(), 'gro_invoke_errors_'));
		onTestFinished(() => rm(dir, { recursive: true, force: true }));
		// a failing `svelte-kit` CLI, what `sveltekit_sync_if_obviously_needed` runs before any task
		await mkdir(join(dir, 'node_modules/.bin'), { recursive: true });
		const cli = join(dir, 'node_modules/.bin/svelte-kit');
		await writeFile(cli, '#!/bin/sh\nexit 1\n');
		await chmod(cli, 0o755);
		await writeFile(
			join(dir, 'package.json'),
			JSON.stringify({ type: 'module', devDependencies: { '@sveltejs/kit': '3' } }) + '\n'
		);

		const result = spawnSync(
			process.execPath,
			spawn_with_loader_args(LOADER_SOURCE_PATH, INVOKE_SOURCE_PATH, [], undefined),
			// the logger goes quiet under `VITEST`, and a child inherits it
			{ cwd: dir, encoding: 'utf8', env: { ...process.env, NO_COLOR: '1', VITEST: '' } }
		);

		assert.equal(result.status, 1);
		assert.include(result.stderr + result.stdout, 'Failed svelte-kit sync');
		assert.notMatch(result.stderr + result.stdout, /^\s+at /m);
	},
	TEST_TIMEOUT_MD
);

const invoke_args = (argv: Array<string>): Array<string> =>
	spawn_with_loader_args(LOADER_SOURCE_PATH, INVOKE_SOURCE_PATH, argv, undefined);

describe('SvelteKit sync without SvelteKit in the dependencies', () => {
	const KIT = { '@sveltejs/kit': '3' };

	// a `svelte-kit` that fails loudly, at `node_modules/.bin` or on a bin directory on `PATH`
	const write_cli = async (dir: string, bin: string): Promise<void> => {
		const path = join(dir, bin, 'svelte-kit');
		await mkdir(dirname(path), { recursive: true });
		await writeFile(path, `#!/bin/sh\ntouch '${join(dir, 'ran')}'\nexit 1\n`);
		await chmod(path, 0o755);
	};

	const run_cli = async (
		argv: Array<string>,
		package_json: string,
		bin: string | null
	): Promise<{ status: number | null; output: string; ran: boolean }> => {
		const dir = await mkdtemp(join(tmpdir(), 'gro_invoke_sync_'));
		onTestFinished(() => rm(dir, { recursive: true, force: true }));
		await writeFile(join(dir, 'package.json'), package_json);
		if (bin !== null) await write_cli(dir, bin);
		const env = cli_env();
		env.PATH = `${join(dir, 'global_bin')}:${env.PATH}`;
		const result = spawnSync(process.execPath, invoke_args(argv), {
			cwd: dir,
			encoding: 'utf8',
			env
		});
		return {
			status: result.status,
			output: result.stderr + result.stdout,
			ran: existsSync(join(dir, 'ran'))
		};
	};

	const to_package_json = (fields: object): string =>
		JSON.stringify({ type: 'module', ...fields }) + '\n';

	const SYNC = ['sync', '--no-package_json', '--no-gen'];

	test(
		'before any task, a CLI runs for no project that does not declare SvelteKit',
		async () => {
			for (const [package_json, bin] of [
				[to_package_json({}), 'node_modules/.bin'],
				[to_package_json({}), 'global_bin'],
				[to_package_json({ peerDependencies: KIT }), 'node_modules/.bin'],
				['null\n', 'node_modules/.bin'],
				['{\n', 'node_modules/.bin']
			] as const) {
				const { status, ran, output } = await run_cli([], package_json, bin);
				assert.isFalse(ran, `${package_json} with ${bin}`);
				assert.strictEqual(status, 0, output);
				assert.notInclude(output, 'Failed to load package.json');
			}
		},
		TEST_TIMEOUT_MD * 2
	);

	test(
		'before any task, a project declaring SvelteKit with no CLI skips, as on a fresh clone',
		async () => {
			const { status, output } = await run_cli([], to_package_json({ devDependencies: KIT }), null);
			assert.strictEqual(status, 0, output);
		},
		TEST_TIMEOUT_MD
	);

	test(
		'`gro sync` passes for a project without SvelteKit, whatever CLI is around',
		async () => {
			const { status, ran, output } = await run_cli(SYNC, to_package_json({}), 'global_bin');
			assert.strictEqual(status, 0, output);
			assert.isFalse(ran);
		},
		TEST_TIMEOUT_MD
	);

	test(
		'`gro sync` gives the install hint for a project declaring SvelteKit with no CLI',
		async () => {
			const { status, output } = await run_cli(
				SYNC,
				to_package_json({ devDependencies: KIT }),
				null
			);
			assert.strictEqual(status, 1, output);
			assert.include(output, 'Failed to find SvelteKit CLI `svelte-kit`');
			assert.include(output, 'install');
		},
		TEST_TIMEOUT_MD
	);
});

// quotes an arg for the bash pipelines below
const to_shell_word = (arg: string): string => `'${arg.replaceAll("'", `'\\''`)}'`;

// the bash pipeline running the source CLI with `argv` and `redirect`, then `rest`
const pipeline = (argv: Array<string>, redirect: string, rest: string): string =>
	[process.execPath, ...invoke_args(argv)].map(to_shell_word).join(' ') + ' ' + redirect + rest;

// the logger goes quiet under `VITEST`, and a child inherits it
const cli_env = (): NodeJS.ProcessEnv => ({ ...process.env, NO_COLOR: '1', VITEST: '' });

const create_project = async (prefix: string): Promise<string> => {
	const dir = await mkdtemp(join(tmpdir(), prefix));
	onTestFinished(() => rm(dir, { recursive: true, force: true }));
	await writeFile(join(dir, 'package.json'), '{"type": "module"}\n');
	// looks synced, so no `svelte-kit sync` runs first: its inherited stdio clears `O_NONBLOCK`
	await mkdir(join(dir, SVELTEKIT_DEV_DIRNAME));
	await mkdir(dirname(join(dir, SVELTEKIT_TSCONFIG_PATH)), { recursive: true });
	await writeFile(join(dir, SVELTEKIT_TSCONFIG_PATH), '{}\n');
	return dir;
};

// a write to each stream that the reader's exit breaks, then time for its `EPIPE` to arrive
const WRITE_TO_GONE_READER = `
	await new Promise((r) => setTimeout(r, 100));
	process.stdout.write('out\\n');
	process.stderr.write('err\\n');
	await new Promise((r) => setTimeout(r, 100));
`;

// runs the task `body` with the stream `redirect` selects piped to a reader that exits at once
const run_task_with_gone_reader = async (
	dir: string,
	body: string,
	redirect: string,
	header = ''
): Promise<number> => {
	await writeFile(
		join(dir, 't.task.ts'),
		`${header}export const task = {run: async () => {${body}}};\n`
	);
	const result = spawnSync(
		'bash',
		[
			'-c',
			pipeline(['task', 't.task.ts'], redirect, ' | head -c 0; echo "${PIPESTATUS[0]}" > "$0"'),
			join(dir, 'status')
		],
		{ cwd: dir, encoding: 'utf8', env: cli_env() }
	);
	assert.equal(result.status, 0);
	return Number((await readFile(join(dir, 'status'), 'utf8')).trim());
};

describe('a thrown non-error reaching the uncaught handler', () => {
	for (const value of ['undefined', 'null']) {
		test(
			`\`throw ${value}\` exits 1`,
			async () => {
				const dir = await create_project('gro_invoke_nullish_');
				await writeFile(
					join(dir, 't.task.ts'),
					`export const task = {run: async () => {
						setTimeout(() => {throw ${value};}, 0);
						await new Promise(() => {});
					}};\n`
				);
				const result = spawnSync(process.execPath, invoke_args(['task', 't.task.ts']), {
					cwd: dir,
					encoding: 'utf8',
					env: cli_env()
				});
				assert.equal(result.status, 1, result.stderr + result.stdout);
				assert.include(result.stderr + result.stdout, `Unknown error: ${value}`);
			},
			TEST_TIMEOUT_MD
		);
	}
});

const BROKEN_STREAMS = [
	['stdout', '2>/dev/null'],
	['stderr', '2>&1 >/dev/null']
] as const;

const OUTCOMES = [
	['a passing task', '', 0],
	['a failing task', `throw new Error('boom');`, 1],
	['a task setting process.exitCode', 'process.exitCode = 3;', 3]
] as const;

describe('a reader that leaves', () => {
	for (const [stream, redirect] of BROKEN_STREAMS) {
		for (const [name, body, expected] of OUTCOMES) {
			test(
				`a broken ${stream} pipe leaves ${name} with its exit code ${expected}`,
				async () => {
					const dir = await create_project('gro_invoke_epipe_');
					const status = await run_task_with_gone_reader(
						dir,
						WRITE_TO_GONE_READER + body,
						redirect
					);
					assert.equal(status, expected);
				},
				TEST_TIMEOUT_MD
			);
		}
	}

	test(
		'a broken pipe lets the run finish, its in-flight child included',
		async () => {
			const dir = await create_project('gro_invoke_epipe_');
			const status = await run_task_with_gone_reader(
				dir,
				`const sh = spawn.program('sh');
				const child = await sh.start(['-c', 'sleep 0.5; echo done > child_done'], {stdio: 'ignore'});
				${WRITE_TO_GONE_READER}
				await child.closed;
				writeFileSync('run_done', '');`,
				'2>/dev/null',
				`import {writeFileSync} from 'node:fs';
				import {spawn} from ${JSON.stringify(SPAWN_SOURCE_PATH)};\n`
			);
			assert.equal(status, 0);
			assert.ok(existsSync(join(dir, 'child_done')), 'the child should have finished');
			assert.ok(existsSync(join(dir, 'run_done')), 'the run should have continued past its write');
		},
		TEST_TIMEOUT_MD
	);

	test(
		'a failure on a broken pipe still ends a child it left running',
		async () => {
			const dir = await create_project('gro_invoke_epipe_');
			const status = await run_task_with_gone_reader(
				dir,
				`const sh = spawn.program('sh');
				const child = await sh.start(['-c', 'exec sleep 30'], {stdio: 'ignore'});
				writeFileSync('child_pid', String(child.pid));
				${WRITE_TO_GONE_READER}
				throw new Error('boom');`,
				'2>/dev/null',
				`import {writeFileSync} from 'node:fs';
				import {spawn} from ${JSON.stringify(SPAWN_SOURCE_PATH)};\n`
			);
			assert.equal(status, 1);
			const pid = Number(await readFile(join(dir, 'child_pid'), 'utf8'));
			assert.ok(Number.isInteger(pid) && pid > 0);
			// SIGKILL's from the exit hook, reaped by init a moment later
			const started = Date.now();
			while (is_running(pid)) {
				assert.ok(Date.now() - started < 5_000, 'the child should be killed by the exit hook');
				await new Promise((r) => setTimeout(r, 50));
			}
		},
		TEST_TIMEOUT_MD
	);
});

const is_running = (pid: number): boolean => {
	try {
		process.kill(pid, 0);
		return true;
	} catch {
		return false;
	}
};

test(
	'a broken stdout pipe on the listing ends the CLI quietly with exit 0 and no stack',
	async () => {
		const dir = await create_project('gro_invoke_epipe_');

		// `head -c 0` exits at once, so the CLI's first write finds no reader
		const result = spawnSync(
			'bash',
			[
				'-c',
				pipeline([], '', ' 2>"$0" | head -c 0; echo "${PIPESTATUS[0]}" > "$0".status'),
				join(dir, 'err')
			],
			{ cwd: dir, encoding: 'utf8', env: cli_env() }
		);

		assert.equal(result.status, 0);
		const status = await readFile(join(dir, 'err.status'), 'utf8');
		const stderr = await readFile(join(dir, 'err'), 'utf8');
		assert.equal(status.trim(), '0');
		assert.notInclude(stderr, 'EPIPE');
		assert.notMatch(stderr, /^\s+at /m);
	},
	TEST_TIMEOUT_MD
);

test(
	'a large error reaches a slow reader on a pipe before the CLI exits',
	async () => {
		const dir = await create_project('gro_invoke_flush_');
		// past a pipe's 64 KiB, so what isn't read yet stays queued in the process
		const size = 400_000;
		await writeFile(
			join(dir, 'big.task.ts'),
			`export const task = {run: () => {throw new Error('x'.repeat(${size}));}};\n`
		);

		const result = spawnSync(
			'bash',
			[
				'-c',
				pipeline(['task', 'big.task.ts'], '2>&1', ' | (sleep 2; wc -c) > "$0"'),
				join(dir, 'count')
			],
			{ cwd: dir, encoding: 'utf8', env: cli_env() }
		);

		assert.equal(result.status, 0);
		const count = Number((await readFile(join(dir, 'count'), 'utf8')).trim());
		assert.ok(count >= size, `expected at least ${size} bytes, got ${count}`);
	},
	TEST_TIMEOUT_MD
);

const run_failing_task = async (
	prefix: string,
	source: string,
	stdio: StdioOptions
): Promise<number | null> => {
	const dir = await create_project(prefix);
	await writeFile(join(dir, 'fails.task.ts'), source);
	const result = spawnSync(process.execPath, invoke_args(['task', 'fails.task.ts']), {
		cwd: dir,
		env: cli_env(),
		stdio,
		timeout: 20_000
	});
	return result.status;
};

test(
	'output that fails even empty writes ends a failed task rather than spinning',
	async () => {
		const full = openSync('/dev/full', 'w');
		onTestFinished(() => closeSync(full));
		const status = await run_failing_task(
			'gro_invoke_dead_output_',
			`export const task = {run: () => {throw new Error('x');}};\n`,
			['ignore', full, full]
		);
		assert.strictEqual(status, 1);
	},
	TEST_TIMEOUT_MD
);

test(
	'a failed task keeps its exit code when the flush can never settle',
	async () => {
		const status = await run_failing_task(
			'gro_invoke_corked_',
			`export const task = {run: () => {process.stdout.cork(); throw new Error('x');}};\n`,
			['ignore', 'pipe', 'pipe']
		);
		assert.strictEqual(status, 1);
	},
	TEST_TIMEOUT_MD
);

test(
	'a reader that never reads can only hold the exit of a failed task for a short bound',
	async () => {
		const dir = await create_project('gro_invoke_flush_');
		// past a pipe's 64 KiB, so what isn't read stays queued in the process
		await writeFile(
			join(dir, 'big.task.ts'),
			`export const task = {run: () => {throw new Error('x'.repeat(400_000));}};\n`
		);

		// the output pipes stay open and unread until the child's gone
		const child = spawn(process.execPath, invoke_args(['task', 'big.task.ts']), {
			cwd: dir,
			env: cli_env(),
			stdio: ['ignore', 'pipe', 'pipe']
		});
		onTestFinished(() => {
			child.kill('SIGKILL');
		});
		const code = await new Promise((resolve) => {
			child.on('exit', resolve);
		});

		assert.equal(code, 1);
	},
	TEST_TIMEOUT_MD
);
