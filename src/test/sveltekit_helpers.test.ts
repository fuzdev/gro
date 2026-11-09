import { assert, describe, test, expect, vi } from 'vitest';
import type { PackageJson } from '@fuzdev/fuz_util/package_json.ts';

import {
	has_sveltekit_app,
	has_sveltekit_library,
	project_has_sveltekit_app
} from '#lib/sveltekit_helpers.ts';
import { LIB_PATH, SVELTE_PACKAGE_DEP_NAME, SVELTEKIT_DEP_NAME } from '#lib/constants.ts';
import {
	create_mock_fs_dir,
	create_mock_fs_paths,
	create_mock_project_fs_paths,
	mock_tree
} from './fs_mocks.ts';
import { create_mock_logger } from './test_helpers.ts';

const to_package_json = (
	deps: Partial<Pick<PackageJson, 'dependencies' | 'devDependencies' | 'peerDependencies'>>
): PackageJson => ({ name: 'a', version: '0', ...deps });

describe('project_has_sveltekit_app', () => {
	const to_project = (tree: Record<string, string>) => {
		const project = create_mock_fs_dir('/project');
		mock_tree(project, tree);
		return create_mock_project_fs_paths(project);
	};

	test('a declared SvelteKit counts in a package.json the schema refuses', async () => {
		const contents = JSON.stringify({ name: 1, devDependencies: { [SVELTEKIT_DEP_NAME]: '3' } });
		expect(await project_has_sveltekit_app(to_project({ 'package.json': contents }))).toBe(true);
	});

	test('no declared SvelteKit is no app', async () => {
		const fs_paths = to_project({ 'package.json': '{"devDependencies": {"typescript": "6"}}' });
		expect(await project_has_sveltekit_app(fs_paths)).toBe(false);
	});

	test('a package.json that does not parse is no app, reported to the log', async () => {
		const log = create_mock_logger();
		expect(await project_has_sveltekit_app(to_project({ 'package.json': '{' }), log)).toBe(false);
		expect(vi.mocked(log.error).mock.calls).toHaveLength(1);
		expect(String(vi.mocked(log.error).mock.calls[0]![0])).toContain('package.json');
	});

	test('a package.json that exists but cannot be read is no app, reported to the log', async () => {
		const log = create_mock_logger();
		// a directory at the path, as a permission error or a symlink loop would be
		expect(await project_has_sveltekit_app(to_project({ 'package.json/x': '' }), log)).toBe(false);
		expect(vi.mocked(log.error).mock.calls).toHaveLength(1);
	});

	test('a missing package.json is no app, with nothing to report', async () => {
		const log = create_mock_logger();
		expect(await project_has_sveltekit_app(to_project({}), log)).toBe(false);
		expect(vi.mocked(log.error).mock.calls).toHaveLength(0);
	});
});

describe('has_sveltekit_app', () => {
	test('detects SvelteKit as a dep or dev dep', () => {
		expect(
			has_sveltekit_app(to_package_json({ dependencies: { [SVELTEKIT_DEP_NAME]: '2' } })).ok
		).toBe(true);
		expect(
			has_sveltekit_app(to_package_json({ devDependencies: { [SVELTEKIT_DEP_NAME]: '2' } })).ok
		).toBe(true);
	});

	test('is not detected with no SvelteKit dependency', () => {
		expect(has_sveltekit_app(to_package_json({})).ok).toBe(false);
	});

	// A package peered on SvelteKit is built to work with one, not to be one -
	// counting the peer would run `vite build` over a library that has no app.
	test('ignores a peer dependency', () => {
		expect(
			has_sveltekit_app(to_package_json({ peerDependencies: { [SVELTEKIT_DEP_NAME]: '2' } })).ok
		).toBe(false);
	});
});

// a project whose tree holds `paths`
const to_project_fs_paths = (paths: Array<string>) => {
	const project = create_mock_fs_dir('/project');
	mock_tree(project, Object.fromEntries(paths.map((p) => [p, ''])));
	return create_mock_project_fs_paths(project);
};

describe('has_sveltekit_library', () => {
	// the mock throws on any op, so these checks read nothing
	test('needs SvelteKit first', async () => {
		const result = await has_sveltekit_library(
			create_mock_fs_paths(),
			to_package_json({ devDependencies: { [SVELTE_PACKAGE_DEP_NAME]: '2' } })
		);
		assert(!result.ok);
		expect(result.message).toContain(SVELTEKIT_DEP_NAME);
	});

	// The packaging dep is checked before the lib directory so a project that isn't a library
	// never reaches the filesystem, the only one of the three checks that costs anything.
	test('needs the packaging dep, and reports it before checking the lib directory', async () => {
		const result = await has_sveltekit_library(
			create_mock_fs_paths(),
			to_package_json({ devDependencies: { [SVELTEKIT_DEP_NAME]: '2' } })
		);
		assert(!result.ok);
		expect(result.message).toContain(SVELTE_PACKAGE_DEP_NAME);
	});

	test('detects a library with its lib directory', async () => {
		const result = await has_sveltekit_library(
			to_project_fs_paths([LIB_PATH + '/']),
			to_package_json({
				devDependencies: { [SVELTEKIT_DEP_NAME]: '2', [SVELTE_PACKAGE_DEP_NAME]: '2' }
			})
		);
		expect(result.ok).toBe(true);
	});

	test('needs the lib directory', async () => {
		const result = await has_sveltekit_library(
			to_project_fs_paths(['src/routes/']),
			to_package_json({
				devDependencies: { [SVELTEKIT_DEP_NAME]: '2', [SVELTE_PACKAGE_DEP_NAME]: '2' }
			})
		);
		assert(!result.ok);
		expect(result.message).toContain(LIB_PATH);
	});

	test('ignores peer dependencies', async () => {
		const result = await has_sveltekit_library(
			create_mock_fs_paths(),
			to_package_json({
				devDependencies: { [SVELTEKIT_DEP_NAME]: '2' },
				peerDependencies: { [SVELTE_PACKAGE_DEP_NAME]: '2' }
			})
		);
		assert(!result.ok);
		expect(result.message).toContain(SVELTE_PACKAGE_DEP_NAME);
	});
});
