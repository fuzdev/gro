import { describe, test, assert, beforeEach, afterEach } from 'vitest';
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

import * as build_cache from '#lib/build_cache.ts';
import type { FsPaths } from '#lib/fs_paths.ts';

import {
	create_mock_build_cache_metadata,
	create_mock_logger
} from './build_cache_test_helpers.ts';
import { create_project_fs_paths } from './fs_test_helpers.ts';

/*

The build cache through symlinks that leave the project, on a real temp project
that the `FsPaths` resolves every relative path beneath, so no op reaches the repo.

*/

let base: string;
let project: string;
let outside: string;
let fs_paths: FsPaths;

beforeEach(async () => {
	base = realpathSync(mkdtempSync(join(tmpdir(), 'gro-build-cache-symlinks-')));
	project = join(base, 'project');
	outside = join(base, 'outside');
	mkdirSync(project);
	mkdirSync(outside);
	fs_paths = await create_project_fs_paths(project);
	assert.strictEqual(fs_paths.project.path, project);
});

afterEach(() => {
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
		assert.deepEqual(await build_cache.discover_build_output_dirs(fs_paths), ['build']);
	});

	test('validates what it collected', async () => {
		link_build_out();
		const outputs = await build_cache.collect_build_outputs(fs_paths, ['build']);
		assert.deepEqual(
			outputs.map((o) => o.path),
			['build/index.html']
		);
		const metadata = create_mock_build_cache_metadata({ outputs });
		assert.ok(await build_cache.validate_build_cache(fs_paths, metadata));

		writeFileSync(join(outside, 'build/index.html'), '<html>changed</html>');
		assert.ok(!(await build_cache.validate_build_cache(fs_paths, metadata)));
	});

	test('an output path naming something outside the project is an invalid cache', async () => {
		link_build_out();
		const [output] = await build_cache.collect_build_outputs(fs_paths, ['build']);
		for (const path of ['../outside/build/index.html', join(outside, 'build/index.html')]) {
			const metadata = create_mock_build_cache_metadata({ outputs: [{ ...output!, path }] });
			assert.ok(!(await build_cache.validate_build_cache(fs_paths, metadata)), path);
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
		await build_cache.save_build_cache_metadata({ fs_paths, log: create_mock_logger() }, metadata);
		assert.ok(existsSync(join(outside, 'gro/build.json')));
		assert.deepEqual(await build_cache.load_build_cache_metadata(fs_paths), metadata);
	});

	test('removes invalid metadata in the directory it points at, keeping the link', async () => {
		writeFileSync(join(outside, 'gro/build.json'), 'invalid json{');
		assert.isNull(await build_cache.load_build_cache_metadata(fs_paths));
		assert.ok(!existsSync(join(outside, 'gro/build.json')));
		assert.ok(lstatSync(join(project, '.gro')).isSymbolicLink());
	});
});

describe('.gro/build.json symlinked out of the project', () => {
	test('removes the link to invalid metadata, never the file it points at', async () => {
		mkdirSync(join(project, '.gro'));
		writeFileSync(join(outside, 'other.json'), 'invalid json{');
		symlinkSync(join(outside, 'other.json'), join(project, '.gro/build.json'));

		assert.isNull(await build_cache.load_build_cache_metadata(fs_paths));
		assert.ok(!existsSync(join(project, '.gro/build.json')));
		assert.strictEqual(readFileSync(join(outside, 'other.json'), 'utf8'), 'invalid json{');
	});

	test('loads valid metadata through the link', async () => {
		mkdirSync(join(project, '.gro'));
		const metadata = create_mock_build_cache_metadata();
		writeFileSync(join(outside, 'other.json'), JSON.stringify(metadata));
		symlinkSync(join(outside, 'other.json'), join(project, '.gro/build.json'));

		assert.deepEqual(await build_cache.load_build_cache_metadata(fs_paths), metadata);
	});
});
