import { describe, test, expect, assert, afterEach } from 'vitest';
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import {
	to_install_program_options,
	is_etarget_error,
	install_with_cache_healing,
	install_with_cache_healing_or_throw,
	gro_package_versions_load,
	gro_package_versions_diff,
	type NpmCommandResult,
	type NpmCommandRunner
} from '#lib/npm_install_helpers.ts';
import { TaskError } from '#lib/task.ts';
import type { FsPaths } from '#lib/fs_paths.ts';
import { create_project_fs_paths } from './fs_test_helpers.ts';
import { create_mock_spawn, type MockSpawn } from './spawn_mocks.ts';

describe('to_install_program_options', () => {
	test('forwards the whole environment but the pruning names', () => {
		expect(to_install_program_options()).toEqual({
			pass_env: 'all',
			omit_env: ['NODE_ENV', 'npm_config_production', 'npm_config_omit']
		});
	});
});

describe('is_etarget_error', () => {
	test('detects the ETARGET shapes from message or stderr', () => {
		expect(is_etarget_error('', 'npm error code ETARGET')).toBe(true);
		expect(is_etarget_error('code ETARGET', '')).toBe(true);
		expect(is_etarget_error('', 'notarget')).toBe(true);
		expect(is_etarget_error('', 'No matching version found for @scope/pkg@1.2.3')).toBe(true);
	});

	test('is case-insensitive', () => {
		expect(is_etarget_error('', 'etarget')).toBe(true);
	});

	test('does not match unrelated failures', () => {
		expect(is_etarget_error('code 1', 'npm error EACCES permission denied')).toBe(false);
		expect(is_etarget_error('', '')).toBe(false);
	});
});

/**
 * Builds a fake `NpmCommandRunner` that returns the given results in order and
 * records every call. Missing results default to a successful run.
 */
const make_run = (
	results: Array<{ ok: boolean; stderr?: string; detail?: string }>
): {
	run: NpmCommandRunner;
	calls: Array<{ command: string; args: Array<string> }>;
} => {
	const calls: Array<{ command: string; args: Array<string> }> = [];
	let i = 0;
	const run: NpmCommandRunner = async (program, args) => {
		calls.push({ command: program.name, args: [...args] });
		const r = results[i++] ?? { ok: true };
		const result: NpmCommandResult = {
			ok: r.ok,
			stderr: r.stderr ?? '',
			detail: r.detail ?? (r.ok ? 'ok' : 'code 1')
		};
		return result;
	};
	return { run, calls };
};

/**
 * A mock spawn that resolves any name, recording the program options each `program` call got.
 */
const make_spawn = (): MockSpawn & { program_options: Array<unknown> } => {
	const mock = create_mock_spawn();
	mock.available = () => true;
	const program_options: Array<unknown> = [];
	const inner_program = mock.program;
	mock.program = (name, options) => {
		program_options.push(options);
		return inner_program(name, options);
	};
	return Object.assign(mock, { program_options });
};

describe('install_with_cache_healing', () => {
	test('succeeds on the first attempt without cache cleaning', async () => {
		const { run, calls } = make_run([{ ok: true }]);
		const result = await install_with_cache_healing(make_spawn(), 'npm', { run });
		expect(result.ok).toBe(true);
		expect(calls).toHaveLength(1);
		expect(calls[0]!.args).toEqual(['install']);
	});

	test('returns a non-ETARGET failure immediately without healing', async () => {
		const { run, calls } = make_run([{ ok: false, stderr: 'npm error EACCES permission denied' }]);
		const result = await install_with_cache_healing(make_spawn(), 'npm', { run });
		expect(result.ok).toBe(false);
		expect(calls).toHaveLength(1);
	});

	test('heals an ETARGET failure: cache clean then retry', async () => {
		const { run, calls } = make_run([
			{ ok: false, stderr: 'npm error code ETARGET' },
			{ ok: true }, // cache clean
			{ ok: true } // retry install
		]);
		const result = await install_with_cache_healing(make_spawn(), 'npm', { run });
		expect(result.ok).toBe(true);
		expect(calls).toHaveLength(3);
		expect(calls[1]!.args).toEqual(['cache', 'clean', '--force']);
		expect(calls[2]!.args).toEqual(['install']);
	});

	test('returns the retry failure when the heal does not fix it', async () => {
		const { run, calls } = make_run([
			{ ok: false, stderr: 'ETARGET' },
			{ ok: true }, // cache clean
			{ ok: false, stderr: 'ETARGET still', detail: 'code 1' } // retry install
		]);
		const result = await install_with_cache_healing(make_spawn(), 'npm', { run });
		expect(result.ok).toBe(false);
		expect(calls).toHaveLength(3);
	});

	test('reports a cache-clean failure and does not retry the install', async () => {
		const { run, calls } = make_run([
			{ ok: false, stderr: 'ETARGET' },
			{ ok: false, detail: 'code 1' } // cache clean fails
		]);
		const result = await install_with_cache_healing(make_spawn(), 'npm', { run });
		expect(result.ok).toBe(false);
		expect(result.detail).toContain('cache clean');
		expect(calls).toHaveLength(2);
	});

	test('builds the program with the whole environment but the pruning names', async () => {
		const { run } = make_run([{ ok: true }]);
		const spawn = make_spawn();
		await install_with_cache_healing(spawn, 'npm', { run });
		expect(spawn.program_options).toEqual([
			{
				pass_env: 'all',
				omit_env: ['NODE_ENV', 'npm_config_production', 'npm_config_omit']
			}
		]);
	});

	test('pins one program for the install, the cache clean, and the retry', async () => {
		const { run, calls } = make_run([{ ok: false, stderr: 'ETARGET' }, { ok: true }, { ok: true }]);
		const spawn = make_spawn();
		await install_with_cache_healing(spawn, 'npm', { run });
		expect(spawn.program_options).toHaveLength(1);
		expect(calls.map((c) => c.args[0])).toEqual(['install', 'cache', 'install']);
	});

	test('a program that cannot be found is a failed result, never healed', async () => {
		const spawn = create_mock_spawn(); // `npm` isn't available
		const result = await install_with_cache_healing(spawn, 'npm');
		assert(!result.ok);
		expect(result.detail).toContain('not_found');
		expect(spawn.calls).toEqual([]);
	});

	test('appends install_args and reuses them on the cache-heal retry', async () => {
		const { run, calls } = make_run([
			{ ok: false, stderr: 'ETARGET' },
			{ ok: true }, // cache clean
			{ ok: true } // retry install
		]);
		const result = await install_with_cache_healing(make_spawn(), 'npm', {
			run,
			install_args: ['foo@latest', '--force']
		});
		expect(result.ok).toBe(true);
		expect(calls[0]!.args).toEqual(['install', 'foo@latest', '--force']);
		expect(calls[1]!.args).toEqual(['cache', 'clean', '--force']);
		expect(calls[2]!.args).toEqual(['install', 'foo@latest', '--force']);
	});

	test('only npm self-heals — a non-npm pm_cli surfaces ETARGET without cache cleaning', async () => {
		const { run, calls } = make_run([{ ok: false, stderr: 'npm error code ETARGET' }]);
		const result = await install_with_cache_healing(make_spawn(), 'some-other-cli', { run });
		expect(result.ok).toBe(false);
		expect(calls).toHaveLength(1); // no cache clean, no retry
	});
});

describe('install_with_cache_healing_or_throw', () => {
	test('resolves without throwing on success', async () => {
		const { run } = make_run([{ ok: true }]);
		await expect(
			install_with_cache_healing_or_throw(make_spawn(), 'npm', { run })
		).resolves.toBeUndefined();
	});

	test('throws a TaskError naming the command, context, and detail on failure', async () => {
		const { run } = make_run([{ ok: false, detail: 'code 1' }]);
		const promise = install_with_cache_healing_or_throw(make_spawn(), 'npm', {
			run,
			install_args: ['foo@latest'],
			context: 'after version bump'
		});
		await expect(promise).rejects.toBeInstanceOf(TaskError);
		await expect(promise).rejects.toThrow(
			'Failed `npm install foo@latest` after version bump: code 1'
		);
	});
});

const temp_dirs: Array<string> = [];
afterEach(async () => {
	await Promise.all(temp_dirs.splice(0).map((dir) => rm(dir, { recursive: true, force: true })));
});

const write_package_json = async (dir: string, data: object): Promise<void> => {
	await mkdir(dir, { recursive: true });
	await writeFile(join(dir, 'package.json'), JSON.stringify(data));
};

/**
 * Lays out a project with Gro installed in `node_modules`, its dependency
 * `dep_a` hoisted to the project, and its peer `peer_b` nested under Gro.
 * Returns the Gro package directory and an `FsPaths` on the project.
 */
const create_project = async (): Promise<{ root: string; gro_dir: string; fs_paths: FsPaths }> => {
	const root = await mkdtemp(join(tmpdir(), 'gro_install_'));
	temp_dirs.push(root);
	const gro_dir = join(root, 'node_modules/@fuzdev/gro');
	await write_package_json(gro_dir, {
		name: '@fuzdev/gro',
		version: '1.0.0',
		dependencies: { dep_a: '^1' },
		peerDependencies: { peer_b: '^2', missing_c: '^3' }
	});
	await write_package_json(join(root, 'node_modules/dep_a'), { version: '1.0.0' });
	await write_package_json(join(gro_dir, 'node_modules/peer_b'), { version: '2.0.0' });
	return { root, gro_dir, fs_paths: await create_project_fs_paths(root) };
};

describe('gro_package_versions_load', () => {
	test('reads Gro and its deps and peers, searching up from the Gro directory', async () => {
		const { gro_dir, fs_paths } = await create_project();
		const versions = await gro_package_versions_load(fs_paths, gro_dir);
		expect(Object.fromEntries(versions)).toEqual({
			'@fuzdev/gro': '1.0.0',
			dep_a: '1.0.0',
			peer_b: '2.0.0',
			missing_c: null
		});
	});

	test('a package nested under Gro shadows a hoisted one', async () => {
		const { root, gro_dir, fs_paths } = await create_project();
		await write_package_json(join(root, 'node_modules/peer_b'), { version: '9.0.0' });
		const versions = await gro_package_versions_load(fs_paths, gro_dir);
		assert.strictEqual(versions.get('peer_b'), '2.0.0');
	});

	test('reads the extra names beside the ones Gro declares', async () => {
		const { root, gro_dir, fs_paths } = await create_project();
		await write_package_json(join(root, 'node_modules/extra'), { version: '4.0.0' });
		const versions = await gro_package_versions_load(fs_paths, gro_dir, ['extra']);
		assert.strictEqual(versions.get('extra'), '4.0.0');
		assert.strictEqual(versions.get('dep_a'), '1.0.0');
	});
});

describe('gro_package_versions_diff', () => {
	test('lists changed, added, and removed packages, sorted', () => {
		const before = new Map([
			['b', '1.0.0'],
			['a', '1.0.0'],
			['same', '1.0.0'],
			['removed', '1.0.0']
		]);
		const after = new Map<string, string | null>([
			['b', '2.0.0'],
			['a', null],
			['same', '1.0.0'],
			['added', '1.0.0']
		]);
		expect(gro_package_versions_diff(before, after)).toEqual([
			'a 1.0.0 → none',
			'added none → 1.0.0',
			'b 1.0.0 → 2.0.0',
			'removed 1.0.0 → none'
		]);
	});

	test('is empty when nothing changed', () => {
		const versions = new Map([['a', '1.0.0']]);
		expect(gro_package_versions_diff(versions, new Map(versions))).toEqual([]);
	});
});

describe('install_with_cache_healing_or_throw with throw_if_gro_packages_changed', () => {
	test('throws a TaskError naming each package the install changed', async () => {
		const { root, gro_dir, fs_paths } = await create_project();
		const run: NpmCommandRunner = async () => {
			await write_package_json(gro_dir, { name: '@fuzdev/gro', version: '1.1.0' });
			await write_package_json(join(root, 'node_modules/dep_a'), { version: '1.2.0' });
			return { ok: true, stderr: '', detail: '' };
		};
		const promise = install_with_cache_healing_or_throw(make_spawn(), 'npm', {
			run,
			gro_dir,
			throw_if_gro_packages_changed: true,
			fs_paths
		});
		await expect(promise).rejects.toBeInstanceOf(TaskError);
		await expect(promise).rejects.toThrow(/re-run the command/);
		await expect(promise).rejects.toThrow('@fuzdev/gro 1.0.0 → 1.1.0');
		await expect(promise).rejects.toThrow('dep_a 1.0.0 → 1.2.0');
		// unchanged, though the upgraded Gro no longer declares it
		await expect(promise).rejects.not.toThrow('peer_b');
	});

	test('a failed install throws the install error, not a version change', async () => {
		const { gro_dir, fs_paths } = await create_project();
		const { run } = make_run([{ ok: false, detail: 'code 1' }]);
		await expect(
			install_with_cache_healing_or_throw(make_spawn(), 'npm', {
				run,
				gro_dir,
				throw_if_gro_packages_changed: true,
				fs_paths
			})
		).rejects.toThrow('Failed `npm install`: code 1');
	});

	test('resolves when the install left Gro and its deps unchanged', async () => {
		const { gro_dir, fs_paths } = await create_project();
		const { run } = make_run([{ ok: true }]);
		await expect(
			install_with_cache_healing_or_throw(make_spawn(), 'npm', {
				run,
				gro_dir,
				throw_if_gro_packages_changed: true,
				fs_paths
			})
		).resolves.toBeUndefined();
	});

	test('ignores changes when the option is off', async () => {
		const { gro_dir } = await create_project();
		const run: NpmCommandRunner = async () => {
			await write_package_json(gro_dir, { name: '@fuzdev/gro', version: '1.1.0' });
			return { ok: true, stderr: '', detail: '' };
		};
		await expect(
			install_with_cache_healing_or_throw(make_spawn(), 'npm', { run })
		).resolves.toBeUndefined();
	});
});
