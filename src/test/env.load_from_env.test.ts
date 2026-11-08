import { describe, test, assert, beforeEach, afterEach } from 'vitest';
import { symlinkSync } from 'node:fs';
import { join } from 'node:path';

import { fs, type FsDir } from '#lib/fs.ts';
import { is_fs_error } from '#lib/fs_error.ts';
import { create_fs_paths, type FsPaths } from '#lib/fs_paths.ts';
import { load_env, load_from_env } from '#lib/env.ts';

const KEY = 'GRO_TEST_LOAD_FROM_ENV_KEY';

let temp: FsDir & AsyncDisposable;
// over the temp dir as the project, so a relative path resolves beneath it, never the cwd
let fs_paths: FsPaths;

beforeEach(async () => {
	delete process.env[KEY];
	temp = await fs.temp_dir();
	fs_paths = create_fs_paths({ ...fs, project: temp });
});

afterEach(async () => {
	delete process.env[KEY];
	await temp[Symbol.asyncDispose]();
});

describe('load_from_env', () => {
	test("reads the project's `.env`", async () => {
		await temp.write('.env', `${KEY}=local`);
		assert.strictEqual(await load_from_env(fs_paths, KEY), 'local');
	});

	test('never reads a parent directory `.env` by default', async () => {
		await temp.write('.env', `${KEY}=parent`);
		await temp.mkdir('child');
		const child_paths = create_fs_paths({ ...fs, project: temp.dir('child') });
		assert.strictEqual(await load_from_env(child_paths, KEY), undefined);
	});

	test('process.env wins over the files', async () => {
		await temp.write('.env', `${KEY}=local`);
		process.env[KEY] = 'ambient';
		assert.strictEqual(await load_from_env(fs_paths, KEY, [join(temp.path, '.env')]), 'ambient');
	});

	test('reads explicit paths in order, skipping a missing one', async () => {
		await temp.write('a.env', `${KEY}=`);
		await temp.write('b.env', `${KEY}=b`);
		const paths = ['a.env', 'missing.env', 'b.env'].map((p) => join(temp.path, p));
		assert.strictEqual(await load_from_env(fs_paths, KEY, paths), 'b');
	});

	test('follows a symlinked env file', async () => {
		await temp.mkdir('elsewhere');
		await temp.write('elsewhere/real.env', `${KEY}=linked`);
		symlinkSync(join(temp.path, 'elsewhere/real.env'), join(temp.path, 'link.env'));
		assert.strictEqual(await load_from_env(fs_paths, KEY, [join(temp.path, 'link.env')]), 'linked');
	});

	test('an env file that can not be read throws', async () => {
		await temp.mkdir('dir.env');
		try {
			await load_from_env(fs_paths, KEY, [join(temp.path, 'dir.env')]);
			assert.fail('expected a throw');
		} catch (error) {
			assert.ok(is_fs_error(error, 'is_a_directory'), String(error));
		}
	});
});

describe('load_env', () => {
	test('later files win and the ambient env wins over them', async () => {
		await temp.write('.env', `${KEY}=base\nGRO_TEST_LOAD_ENV_OTHER=base`);
		await temp.write('.env.development', `${KEY}=dev`);
		const env = await load_env(fs_paths, { dev: true, env_dir: temp.path, ambient_env: {} });
		assert.strictEqual(env[KEY], 'dev');
		assert.strictEqual(env.GRO_TEST_LOAD_ENV_OTHER, 'base');
		const ambient = await load_env(fs_paths, {
			dev: true,
			env_dir: temp.path,
			ambient_env: { [KEY]: 'a' }
		});
		assert.strictEqual(ambient[KEY], 'a');
	});

	test('resolves a relative `env_dir` against the project', async () => {
		await temp.mkdir('envs');
		await temp.write('envs/.env', `${KEY}=nested`);
		const env = await load_env(fs_paths, { dev: true, env_dir: 'envs', ambient_env: {} });
		assert.strictEqual(env[KEY], 'nested');
	});

	test('skips the missing files', async () => {
		const env = await load_env(fs_paths, { dev: false, env_dir: temp.path, ambient_env: {} });
		assert.deepEqual(env, {});
	});
});
