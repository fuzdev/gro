import { describe, test, assert, vi, beforeEach } from 'vitest';
import { existsSync, readFileSync } from 'node:fs';

import { load_from_env } from '#lib/env.ts';

vi.mock('node:fs', () => ({
	existsSync: vi.fn(),
	readFileSync: vi.fn()
}));

const KEY = 'GRO_TEST_LOAD_FROM_ENV_KEY';

// a virtual filesystem of env files keyed by the path `load_from_env` asks for
const files: Map<string, string> = new Map();

beforeEach(() => {
	files.clear();
	delete process.env[KEY];
	vi.mocked(existsSync).mockImplementation((path) => files.has(String(path)));
	vi.mocked(readFileSync).mockImplementation((path) => {
		const content = files.get(String(path));
		if (content === undefined) throw Error('missing ' + String(path));
		return content;
	});
});

describe('load_from_env', () => {
	test('reads the project `.env`', () => {
		files.set('.env', `${KEY}=local`);
		assert.strictEqual(load_from_env(KEY), 'local');
	});

	test('never reads a parent directory `.env` by default', () => {
		files.set('../.env', `${KEY}=parent`);
		assert.strictEqual(load_from_env(KEY), undefined);
		for (const [path] of vi.mocked(existsSync).mock.calls) {
			assert.notStrictEqual(String(path), '../.env');
		}
	});

	test('process.env wins over the files', () => {
		files.set('.env', `${KEY}=local`);
		process.env[KEY] = 'ambient';
		try {
			assert.strictEqual(load_from_env(KEY), 'ambient');
		} finally {
			delete process.env[KEY];
		}
	});

	test('reads explicit paths in order', () => {
		files.set('a.env', `${KEY}=`);
		files.set('b.env', `${KEY}=b`);
		assert.strictEqual(load_from_env(KEY, ['a.env', 'b.env']), 'b');
	});
});
