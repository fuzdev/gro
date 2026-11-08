import { describe, test, assert, beforeEach, afterEach } from 'vitest';
import { symlinkSync } from 'node:fs';
import { join } from 'node:path';

import { fs, is_fs_error, type FsDir } from '#lib/fs.ts';
import { load_env, load_from_env } from '#lib/env.ts';

const KEY = 'GRO_TEST_LOAD_FROM_ENV_KEY';

let temp: FsDir & AsyncDisposable;
let cwd: string;

beforeEach(async () => {
	delete process.env[KEY];
	temp = await fs.temp_dir();
	cwd = process.cwd();
});

afterEach(async () => {
	process.chdir(cwd);
	delete process.env[KEY];
	await temp[Symbol.asyncDispose]();
});

describe('load_from_env', () => {
	test('reads the cwd `.env`', async () => {
		await temp.write('.env', `${KEY}=local`);
		process.chdir(temp.path);
		assert.strictEqual(await load_from_env(KEY), 'local');
	});

	test('never reads a parent directory `.env` by default', async () => {
		await temp.write('.env', `${KEY}=parent`);
		await temp.mkdir('child');
		process.chdir(join(temp.path, 'child'));
		assert.strictEqual(await load_from_env(KEY), undefined);
	});

	test('process.env wins over the files', async () => {
		await temp.write('.env', `${KEY}=local`);
		process.env[KEY] = 'ambient';
		assert.strictEqual(await load_from_env(KEY, [join(temp.path, '.env')]), 'ambient');
	});

	test('reads explicit paths in order, skipping a missing one', async () => {
		await temp.write('a.env', `${KEY}=`);
		await temp.write('b.env', `${KEY}=b`);
		const paths = ['a.env', 'missing.env', 'b.env'].map((p) => join(temp.path, p));
		assert.strictEqual(await load_from_env(KEY, paths), 'b');
	});

	test('follows a symlinked env file', async () => {
		await temp.mkdir('elsewhere');
		await temp.write('elsewhere/real.env', `${KEY}=linked`);
		symlinkSync(join(temp.path, 'elsewhere/real.env'), join(temp.path, 'link.env'));
		assert.strictEqual(await load_from_env(KEY, [join(temp.path, 'link.env')]), 'linked');
	});

	test('an env file that can not be read throws', async () => {
		await temp.mkdir('dir.env');
		try {
			await load_from_env(KEY, [join(temp.path, 'dir.env')]);
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
		const env = await load_env({ dev: true, env_dir: temp.path, ambient_env: {} });
		assert.strictEqual(env[KEY], 'dev');
		assert.strictEqual(env.GRO_TEST_LOAD_ENV_OTHER, 'base');
		const ambient = await load_env({ dev: true, env_dir: temp.path, ambient_env: { [KEY]: 'a' } });
		assert.strictEqual(ambient[KEY], 'a');
	});

	test('skips the missing files', async () => {
		const env = await load_env({ dev: false, env_dir: temp.path, ambient_env: {} });
		assert.deepEqual(env, {});
	});
});
