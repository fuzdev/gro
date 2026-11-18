import { describe, test, assert } from 'vitest';
import { readFileSync } from 'node:fs';
import { createRequire, isBuiltin } from 'node:module';
import { dirname, join, relative, resolve } from 'node:path';

import { paths } from '#lib/paths.ts';
import { parse_imports } from '#lib/internal/parse_imports.ts';
import { to_lib_url, use_fixture_runner } from './host_fixture_helpers.ts';

/*

Library modules a project imports as a package on the runtime, where a package holds no grant
and `src/lib` reaches its host modules by relative path, so they'd load the Node files.
Each one's static import closure, `import type` erased, must hold no `node:` builtin
and no host module's Node file, beside the grant-free error modules,
and loading it must read nothing of the process.
A module freed for the runtime joins `PACKAGE_MODULES`.

*/

const PACKAGE_MODULES = ['fs_paths.ts', 'git.ts', 'task_error.ts'];

// the host modules, whose Node files a relative import from `src/lib` loads, granted or not;
// their error modules aren't here, grant-free classes a package module may import
const HOST_MODULES: ReadonlySet<string> = new Set(
	[
		'fs.ts',
		'spawn.ts',
		'env.ts',
		'args.ts',
		'exit.ts',
		'stdin.ts',
		'stdout.ts',
		'stderr.ts',
		'cwd.ts',
		'pid.ts',
		'runtime_version.ts',
		'blake3.ts',
		'net.ts',
		'watch.ts'
	].map((name) => join(paths.lib, name))
);

/**
 * The value import specifiers of the module at `id`, failing on a parse error.
 */
const to_specifiers = (id: string): Array<string> => {
	const parsed = parse_imports(id, readFileSync(id, 'utf8'));
	if (!parsed.ok) assert.fail(`${id} doesn't parse: ${parsed.error.message}`);
	return parsed.specifiers;
};

/**
 * What the static import closure of the module at `entry` reaches that a package module can't,
 * each with the chain from `entry` that reaches it.
 */
const to_closure_violations = (entry: string): Array<string> => {
	const parents: Map<string, string | null> = new Map([[entry, null]]);
	const queue = [entry];
	const violations: Array<string> = [];
	const to_chain = (id: string): string => {
		const chain: Array<string> = [];
		for (let at: string | null = id; at !== null; at = parents.get(at) ?? null) {
			chain.push(relative(paths.root, at));
		}
		return chain.join(' <- ');
	};
	for (let id = queue.shift(); id !== undefined; id = queue.shift()) {
		for (const specifier of to_specifiers(id)) {
			if (specifier.startsWith('node:') || isBuiltin(specifier)) {
				violations.push(`${specifier} <- ${to_chain(id)}`);
				continue;
			}
			const target = specifier.startsWith('.')
				? resolve(dirname(id), specifier)
				: createRequire(id).resolve(specifier);
			if (HOST_MODULES.has(target)) {
				violations.push(`${relative(paths.root, target)} <- ${to_chain(id)}`);
				continue;
			}
			if (!parents.has(target)) {
				parents.set(target, id);
				queue.push(target);
			}
		}
	}
	return violations;
};

const { run_fixture_json } = use_fixture_runner('');

describe('library modules a package imports on the runtime', () => {
	for (const name of PACKAGE_MODULES) {
		test(`${name} reaches no \`node:\` builtin or host module`, () => {
			assert.deepEqual(to_closure_violations(join(paths.lib, name)), []);
		});

		test(`${name} reads nothing of the process when it loads`, () => {
			const reads = run_fixture_json(
				`const real = globalThis.process;
				 const reads = new Set();
				 const proxy = new Proxy(real, {
					get: (t, k) => { reads.add(String(k)); return Reflect.get(t, k); }
				 });
				 Object.defineProperty(globalThis, 'process', {
					configurable: true,
					get: () => { reads.add('process'); return proxy; }
				 });
				 await import(${JSON.stringify(to_lib_url(name))});
				 Object.defineProperty(globalThis, 'process', {configurable: true, writable: true, value: real});
				 console.log(JSON.stringify([...reads].sort()));`
			);
			assert.deepEqual(reads, []);
		});
	}
});
