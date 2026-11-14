import { describe, test, assert } from 'vitest';

import { to_lib_url, use_fixture_runner } from './host_fixture_helpers.ts';

/*

Importing one process module reaches only its own part of the process: what each reads of
Node's global `process` when it loads, so importing `pid.ts` reads no environment and makes no
stream, and only `cwd.ts` reads the working directory.

*/

// what each module reads of the global at load, `env.*` its enumeration of the environment
const READS_AT_LOAD: Record<string, Array<string>> = {
	'env.ts': ['env', 'env.*'],
	'args.ts': ['argv', '_eval'],
	'exit.ts': ['exit'],
	'stdin.ts': [],
	'stdout.ts': [],
	'stderr.ts': [],
	'cwd.ts': ['cwd'],
	'pid.ts': ['pid'],
	'runtime_version.ts': ['version'],
	'env_error.ts': [],
	'stdio_error.ts': []
};

const { run_fixture_json } = use_fixture_runner('');

describe('process modules at load', () => {
	for (const [name, expected] of Object.entries(READS_AT_LOAD)) {
		test(`${name} reads only its own part of the process`, () => {
			const reads = run_fixture_json(
				`const real = globalThis.process;
				 const reads = new Set();
				 const env = new Proxy(real.env, {
					get: (t, k) => { reads.add('env.*'); return Reflect.get(t, k); },
					ownKeys: (t) => { reads.add('env.*'); return Reflect.ownKeys(t); }
				 });
				 globalThis.process = new Proxy(real, {
					get: (t, k) => { reads.add(String(k)); return k === 'env' ? env : Reflect.get(t, k); }
				 });
				 await import(${JSON.stringify(to_lib_url(name))});
				 globalThis.process = real;
				 console.log(JSON.stringify([...reads].sort()));`
			);
			assert.deepEqual(reads, [...expected].sort());
		});
	}
});
