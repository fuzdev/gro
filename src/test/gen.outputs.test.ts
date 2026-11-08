import { describe, test, assert, vi, beforeEach, afterEach } from 'vitest';
import {
	existsSync,
	mkdirSync,
	mkdtempSync,
	readdirSync,
	readFileSync,
	realpathSync,
	rmSync,
	symlinkSync,
	writeFileSync
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { create_mock_logger } from './test_helpers.ts';

/*

Hermetic: the cwd moves to a sandbox project and the modules reload,
so `fs.project` is the sandbox, and no gen output reaches the repo.
`process.chdir` needs Vitest's default `forks` pool.

*/

let base: string;
let project: string;
let outside: string;
let cwd: string;
let gen: typeof import('#lib/gen.ts');

beforeEach(async () => {
	cwd = process.cwd();
	base = realpathSync(mkdtempSync(join(tmpdir(), 'gro-gen-outputs-')));
	project = join(base, 'project');
	outside = join(base, 'outside');
	mkdirSync(join(project, 'src/a'), { recursive: true });
	mkdirSync(outside);
	writeFileSync(join(outside, 'existing.txt'), 'outside');
	process.chdir(project);
	vi.resetModules();
	gen = await import('#lib/gen.ts');
	assert.strictEqual((await import('#lib/fs.ts')).fs.project.path, project);
});

afterEach(() => {
	process.chdir(cwd);
	vi.resetModules();
	rmSync(base, { recursive: true, force: true });
});

const origin = (): string => join(project, 'src/a/x.gen.ts');

const to_results = (filename: string): import('#lib/gen.ts').GenResults => {
	const { files } = gen.to_gen_result(origin(), { content: 'generated\n', filename });
	return {
		results: [],
		successes: [{ ok: true, id: origin(), files, elapsed: 0 }],
		failures: [],
		input_count: 1,
		output_count: files.length,
		elapsed: 0
	} as any;
};

const run_gen = async (filename: string): Promise<void> => {
	const results = to_results(filename);
	await gen.write_gen_results(
		results,
		await gen.analyze_gen_results(results),
		create_mock_logger()
	);
};

const assert_refused = async (promise: Promise<unknown>): Promise<void> => {
	try {
		await promise;
		assert.fail('expected a refusal');
	} catch (error) {
		assert.include(String(error), 'resolves outside the project');
	}
};

describe('gen outputs', () => {
	test('a relative filename climbing out of its directory but staying in the project is written', async () => {
		await run_gen('../elsewhere/y.ts');
		assert.strictEqual(readFileSync(join(project, 'src/elsewhere/y.ts'), 'utf8'), 'generated\n');
	});

	test('an absolute filename inside the project is written', async () => {
		await run_gen(join(project, 'src/abs.ts'));
		assert.strictEqual(readFileSync(join(project, 'src/abs.ts'), 'utf8'), 'generated\n');
	});

	test('an output outside the project is refused, relative or absolute, before any read or write', async () => {
		for (const filename of ['../../../outside/new.txt', join(outside, 'existing.txt')]) {
			// `--check` analyzes without writing, so the read is refused too
			await assert_refused(gen.analyze_gen_results(to_results(filename)));
			await assert_refused(run_gen(filename));
		}
		assert.deepEqual(readdirSync(outside), ['existing.txt']);
		assert.strictEqual(readFileSync(join(outside, 'existing.txt'), 'utf8'), 'outside');
	});

	test('an output resolving to the project directory itself is refused as that', async () => {
		try {
			await gen.analyze_gen_results(to_results('../..'));
			assert.fail('expected a refusal');
		} catch (error) {
			assert.include(String(error), 'resolves to the project directory itself');
		}
	});

	test('an output reaching outside through a symlink is refused', async () => {
		symlinkSync(outside, join(project, 'src/out'));
		symlinkSync(join(outside, 'existing.txt'), join(project, 'src/a/linked.ts'));
		await assert_refused(run_gen('../out/new.txt'));
		await assert_refused(run_gen('linked.ts'));
		assert.deepEqual(readdirSync(outside), ['existing.txt']);
		assert.strictEqual(readFileSync(join(outside, 'existing.txt'), 'utf8'), 'outside');
		assert.ok(!existsSync(join(outside, 'new.txt')));
	});
});
