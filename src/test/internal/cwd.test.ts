import { describe, test, assert, beforeEach, afterEach } from 'vitest';
import { spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, realpathSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

/*

Each case loads a module in a child process whose cwd is a sandbox directory
whose path isn't UTF-8, so nothing here runs with the repo as the cwd.

*/

let base: string;

beforeEach(() => {
	base = realpathSync(mkdtempSync(join(tmpdir(), 'gro-cwd-')));
	// `<base>/\xff/proj`, which Node's `process.cwd()` would report as `<base>/\uFFFD/proj`
	mkdirSync(Buffer.concat([Buffer.from(base + '/'), Buffer.from([0xff]), Buffer.from('/proj')]), {
		recursive: true
	});
	// the lossy twin, the directory a lossy cwd would name
	mkdirSync(join(base, '\uFFFD/proj'), { recursive: true });
});

afterEach(() => {
	rmSync(base, { recursive: true, force: true });
});

// runs `node <module>` from `<base>/\xff/proj`, which the shell enters by its bytes
const load_in_lossy_cwd = (module_path: string) =>
	spawnSync(
		'sh',
		['-c', `cd "$(printf '\\377')/proj" && exec "$0" "$1"`, process.execPath, module_path],
		{
			cwd: base,
			encoding: 'utf8'
		}
	);

describe('the checked cwd', () => {
	for (const module of ['src/lib/fs.ts', 'src/lib/paths.ts']) {
		test(`${module} refuses to load in a cwd whose path isn't UTF-8`, () => {
			const result = load_in_lossy_cwd(resolve(module));
			assert.notStrictEqual(result.status, 0, result.stderr);
			assert.include(result.stderr, "the working directory's path isn't UTF-8");
		});
	}
});
