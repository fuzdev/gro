import { describe, test, expect, vi, beforeEach, afterAll } from 'vitest';
import {
	chmodSync,
	mkdirSync,
	mkdtempSync,
	readdirSync,
	realpathSync,
	rmSync,
	statSync,
	symlinkSync,
	writeFileSync
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { gro_plugin_sveltekit_library } from '#lib/gro_plugin_sveltekit_library.ts';
import type { PluginContext } from '#lib/plugin.ts';
import { fs } from '#lib/fs.ts';
import { create_fs_paths } from '#lib/fs_paths.ts';

import { create_mock_plugin_context } from './test_helpers.ts';
import { create_mock_spawn } from './spawn_mocks.ts';

vi.mock('#lib/package_json.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/package_json.ts')>()),
	package_json_load: vi.fn(async () => ({ name: 'a', version: '0', bin: { a: 'dist/a.js' } }))
}));

// the plugin's project, a fresh temp directory, so the bins are real files
const project = realpathSync(mkdtempSync(tmpdir() + '/gro-plugin-bin-test-'));

const spawn = create_mock_spawn();

const to_ctx = async (): Promise<PluginContext> =>
	create_mock_plugin_context({
		dev: false,
		watch: false,
		host: {
			fs_paths: create_fs_paths(
				{ ...fs, project: await fs.open_dir(project) },
				{ cwd: project, home: undefined }
			),
			spawn
		}
	});

const mode_of = (path: string): number => statSync(path).mode & 0o7777;

// a file in the project with an exact mode, whatever the umask
const write_bin = (project: string, path: string, mode: number): void => {
	mkdirSync(join(project, path, '..'), { recursive: true });
	writeFileSync(join(project, path), '#!/usr/bin/env node\n');
	chmodSync(join(project, path), mode);
};

describe('gro_plugin_sveltekit_library adapt', () => {
	beforeEach(() => {
		spawn.reset();
		for (const name of readdirSync(project)) rmSync(join(project, name), { recursive: true });
	});

	afterAll(() => {
		rmSync(project, { recursive: true, force: true });
	});

	test('adds the execute bits to each bin, keeping the rest, and spawns nothing', async () => {
		write_bin(project, 'dist/a.js', 0o644);
		await gro_plugin_sveltekit_library().adapt!(await to_ctx());
		expect(mode_of(join(project, 'dist/a.js'))).toBe(0o755);
		expect(spawn.calls).toEqual([]);
	});

	test('sets every bin of a package with several', async () => {
		const { package_json_load } = vi.mocked(await import('#lib/package_json.ts'));
		package_json_load.mockResolvedValueOnce({
			name: 'a',
			version: '0',
			bin: { a: 'dist/a.js', b: './dist/b.js' }
		});
		write_bin(project, 'dist/a.js', 0o600);
		write_bin(project, 'dist/b.js', 0o640);
		await gro_plugin_sveltekit_library().adapt!(await to_ctx());
		expect(mode_of(join(project, 'dist/a.js'))).toBe(0o711);
		expect(mode_of(join(project, 'dist/b.js'))).toBe(0o751);
	});

	test('drops a setuid, setgid, or sticky bit, keeping the permission bits', async () => {
		write_bin(project, 'dist/a.js', 0o7644);
		await gro_plugin_sveltekit_library().adapt!(await to_ctx());
		expect(mode_of(join(project, 'dist/a.js'))).toBe(0o755);
	});

	test('a bin that is a symlink sets the file it points at', async () => {
		write_bin(project, 'lib/a.js', 0o644);
		mkdirSync(join(project, 'dist'));
		symlinkSync('../lib/a.js', join(project, 'dist/a.js'));
		await gro_plugin_sveltekit_library().adapt!(await to_ctx());
		expect(mode_of(join(project, 'lib/a.js'))).toBe(0o755);
	});

	test('changes nothing for a package with no bin', async () => {
		const { package_json_load } = vi.mocked(await import('#lib/package_json.ts'));
		package_json_load.mockResolvedValueOnce({ name: 'a', version: '0' });
		write_bin(project, 'dist/a.js', 0o644);
		await gro_plugin_sveltekit_library().adapt!(await to_ctx());
		expect(mode_of(join(project, 'dist/a.js'))).toBe(0o644);
		expect(spawn.calls).toEqual([]);
	});

	test('a missing bin is logged, not thrown', async () => {
		const ctx = await to_ctx();
		await gro_plugin_sveltekit_library().adapt!(ctx);
		expect(ctx.log.error).toHaveBeenCalledWith(
			'chmod on bin path dist/a.js failed: not_found: nothing is at dist/a.js'
		);
	});

	test('a bin whose path fails is logged with the fs error', async () => {
		write_bin(project, 'dist', 0o644);
		const ctx = await to_ctx();
		await gro_plugin_sveltekit_library().adapt!(ctx);
		expect(vi.mocked(ctx.log.error).mock.calls[0]![0]).toMatch(
			/^chmod on bin path dist\/a\.js failed: not_a_directory: /
		);
	});
});
