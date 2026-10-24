import { test, assert, onTestFinished } from 'vitest';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { hash_blake3 } from '@fuzdev/fuz_util/hash_blake3.ts';

import {
	load_build_cache_dependencies,
	NPM_HIDDEN_LOCKFILE_PATH
} from '#lib/build_cache_dependencies.ts';
import { package_json_load_for_gro } from '#lib/package_json.ts';

const create_temp_dir = async (): Promise<string> => {
	const dir = await mkdtemp(join(tmpdir(), 'gro_build_cache_dependencies_'));
	onTestFinished(() => rm(dir, { recursive: true, force: true }));
	return dir;
};

const write_lockfile = async (dir: string, contents: string): Promise<void> => {
	await mkdir(join(dir, 'node_modules'), { recursive: true });
	await writeFile(join(dir, NPM_HIDDEN_LOCKFILE_PATH), contents);
};

test('reads the running Gro and Node versions', async () => {
	const dir = await create_temp_dir();
	const deps = await load_build_cache_dependencies(dir);
	const gro_package_json = await package_json_load_for_gro();
	assert.equal(deps.gro_version, gro_package_json.version);
	assert.equal(deps.node_version, process.version);
});

test('package_lock_hash is null without node_modules/.package-lock.json', async () => {
	const dir = await create_temp_dir();
	const deps = await load_build_cache_dependencies(dir);
	assert.equal(deps.package_lock_hash, null);
});

test('package_lock_hash is the BLAKE3 hash of the hidden lockfile', async () => {
	const dir = await create_temp_dir();
	const contents = '{"name":"a","lockfileVersion":3,"packages":{}}';
	await write_lockfile(dir, contents);
	const deps = await load_build_cache_dependencies(dir);
	assert.equal(deps.package_lock_hash, hash_blake3(contents));
});

test('package_lock_hash changes when the installed packages change', async () => {
	const dir = await create_temp_dir();
	await write_lockfile(dir, '{"packages":{"node_modules/a":{"version":"1.0.0"}}}');
	const before = await load_build_cache_dependencies(dir);
	await write_lockfile(dir, '{"packages":{"node_modules/a":{"version":"1.0.1"}}}');
	const after = await load_build_cache_dependencies(dir);
	assert.ok(before.package_lock_hash);
	assert.notEqual(before.package_lock_hash, after.package_lock_hash);
});
