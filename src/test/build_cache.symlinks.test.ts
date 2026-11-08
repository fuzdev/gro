import { describe, test, assert, vi, beforeEach, afterEach } from 'vitest';
import {
	existsSync,
	lstatSync,
	mkdirSync,
	mkdtempSync,
	readFileSync,
	realpathSync,
	rmSync,
	symlinkSync,
	writeFileSync
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { create_mock_build_cache_metadata } from './build_cache_test_helpers.ts';

/*

The build cache through symlinks that leave the project, on a real project:
the cwd moves to a temp project and the modules reload, so `fs.project` and `paths` are there.
`process.chdir` needs Vitest's default `forks` pool.

*/

let base: string;
let project: string;
let outside: string;
let cwd: string;
let build_cache: typeof import('#lib/build_cache.ts');

beforeEach(async () => {
	cwd = process.cwd();
	base = realpathSync(mkdtempSync(join(tmpdir(), 'gro-build-cache-symlinks-')));
	project = join(base, 'project');
	outside = join(base, 'outside');
	mkdirSync(project);
	mkdirSync(outside);
	process.chdir(project);
	vi.resetModules();
	build_cache = await import('#lib/build_cache.ts');
	// the reloaded modules see the sandbox, so no op reaches the repo
	assert.strictEqual((await import('#lib/fs.ts')).fs.project.path, project);
	assert.strictEqual((await import('#lib/paths.ts')).paths.root, project + '/');
});

afterEach(() => {
	process.chdir(cwd);
	vi.resetModules();
	rmSync(base, { recursive: true, force: true });
});

const link_build_out = (): void => {
	mkdirSync(join(outside, 'build'));
	writeFileSync(join(outside, 'build/index.html'), '<html></html>');
	symlinkSync(join(outside, 'build'), join(project, 'build'));
};

describe('build/ symlinked out of the project', () => {
	test('is discovered', async () => {
		link_build_out();
		assert.deepEqual(await build_cache.discover_build_output_dirs(), ['build']);
	});

	test('validates what it collected', async () => {
		link_build_out();
		const outputs = await build_cache.collect_build_outputs(['build']);
		assert.deepEqual(
			outputs.map((o) => o.path),
			['build/index.html']
		);
		const metadata = create_mock_build_cache_metadata({ outputs });
		assert.ok(await build_cache.validate_build_cache(metadata));

		writeFileSync(join(outside, 'build/index.html'), '<html>changed</html>');
		assert.ok(!(await build_cache.validate_build_cache(metadata)));
	});

	test('an output path naming something outside the project is an invalid cache', async () => {
		link_build_out();
		const [output] = await build_cache.collect_build_outputs(['build']);
		for (const path of ['../outside/build/index.html', join(outside, 'build/index.html')]) {
			const metadata = create_mock_build_cache_metadata({ outputs: [{ ...output!, path }] });
			assert.ok(!(await build_cache.validate_build_cache(metadata)), path);
		}
	});
});

describe('.gro symlinked out of the project', () => {
	beforeEach(() => {
		mkdirSync(join(outside, 'gro'));
		symlinkSync(join(outside, 'gro'), join(project, '.gro'));
	});

	test('loads the metadata saved through it', async () => {
		const metadata = create_mock_build_cache_metadata();
		await build_cache.save_build_cache_metadata(metadata);
		assert.ok(existsSync(join(outside, 'gro/build.json')));
		assert.deepEqual(await build_cache.load_build_cache_metadata(), metadata);
	});

	test('removes invalid metadata in the directory it points at, keeping the link', async () => {
		writeFileSync(join(outside, 'gro/build.json'), 'invalid json{');
		assert.isNull(await build_cache.load_build_cache_metadata());
		assert.ok(!existsSync(join(outside, 'gro/build.json')));
		assert.ok(lstatSync(join(project, '.gro')).isSymbolicLink());
	});
});

describe('.gro/build.json symlinked out of the project', () => {
	test('removes the link to invalid metadata, never the file it points at', async () => {
		mkdirSync(join(project, '.gro'));
		writeFileSync(join(outside, 'other.json'), 'invalid json{');
		symlinkSync(join(outside, 'other.json'), join(project, '.gro/build.json'));

		assert.isNull(await build_cache.load_build_cache_metadata());
		assert.ok(!existsSync(join(project, '.gro/build.json')));
		assert.strictEqual(readFileSync(join(outside, 'other.json'), 'utf8'), 'invalid json{');
	});

	test('loads valid metadata through the link', async () => {
		mkdirSync(join(project, '.gro'));
		const metadata = create_mock_build_cache_metadata();
		writeFileSync(join(outside, 'other.json'), JSON.stringify(metadata));
		symlinkSync(join(outside, 'other.json'), join(project, '.gro/build.json'));

		assert.deepEqual(await build_cache.load_build_cache_metadata(), metadata);
	});
});
