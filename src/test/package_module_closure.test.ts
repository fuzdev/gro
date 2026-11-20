import { describe, test, assert } from 'vitest';
import { readFileSync } from 'node:fs';
import { createRequire, isBuiltin } from 'node:module';
import { dirname, join, relative, resolve } from 'node:path';

import { parse_imports } from '#lib/internal/parse_imports.ts';
import { to_lib_url, use_fixture_runner } from './host_fixture_helpers.ts';
import { paths } from './test_helpers.ts';

/*

Library modules a project imports as a package on the runtime, where a package holds no grant
and `src/lib` reaches its host modules by relative path, so they'd load the Node files.
Each one's static import closure, `import type` erased, must hold no `node:` builtin
and no host module's Node file, beside the grant-free error modules,
and loading it must read nothing of the process.
A module freed for the runtime joins `PACKAGE_MODULES`.

Every module the CLI's entry, `invoke.ts`, reaches by relative import is one of those,
or a root, a host module, Node-only machinery, or an exception, each listed below,
so a module newly reached fails until it's sorted, and an exception that no longer
reaches Node fails until it moves to `PACKAGE_MODULES`.

*/

const PACKAGE_MODULES = [
	'cli.ts',
	'constants.ts',
	'error_detail.ts',
	'fs_paths.ts',
	'fs_search.ts',
	'git.ts',
	'github.ts',
	'gro_builtin_tasks.ts',
	'input_path.ts',
	'internal/code_points.ts',
	'internal/env_name.ts',
	'internal/error_class.ts',
	'internal/error_report.ts',
	'internal/fs_boundary.ts',
	'internal/fs_handles.ts',
	'internal/posix_path.ts',
	'internal/records.ts',
	'modules.ts',
	'paths.ts',
	'task.ts',
	'task_error.ts',
	'task_logging.ts',
	'task_registry.ts'
];

// the package modules the CLI's entry doesn't reach, loaded by tasks and genfiles
const PACKAGE_MODULES_BEYOND_THE_CLI = ['git.ts'];

// the composition roots, which import the host modules and pass what they open down
const ROOTS = [
	'invoke.ts',
	'gro.config.default.ts',
	'gro_plugin_gen.ts',
	'gro_plugin_sveltekit_app.ts',
	'gro_plugin_sveltekit_library.ts'
];

// the grant-free error modules beside the host modules, which a package module may import
const ERROR_MODULES = [
	'env_error.ts',
	'fs_error.ts',
	'net_error.ts',
	'spawn_error.ts',
	'stdio_error.ts'
];

// what only the Node Gro runs, which the runtime replaces rather than loads
const NODE_ONLY = [
	// the checked working directory behind the Node `fs` and `cwd` host modules
	'internal/cwd.ts',
	// the Node `stdout` and `stderr` host modules' streams
	'internal/process_output.ts',
	// the CLI's listeners on the global `process`, which no host module has
	'internal/node_cli_process.ts',
	// Node's project loader, which the runtime's entry hands in instead
	'internal/node_import_module.ts',
	// the Node module graph and the Filer family it imports when first used
	'module_graph_node.ts',
	'internal/filer.ts',
	'internal/watch_dir.ts',
	'internal/parse_imports.ts',
	'resolve_specifier.ts'
];

// library modules whose closure still reaches Node, each with what it reaches
const EXCEPTIONS = [
	// tsv's N-API binding, `@fuzdev/tsv`, which loads through `node:module`
	'dist_rewrite_imports.ts',
	'gen_helpers.ts',
	// the Svelte config, read through Vite, a package on `node:*`
	'svelte_config.ts',
	'gen.ts',
	'run_task.ts',
	// sync stats for the hints of arg parsing, which no sync boundary opens
	'internal/task_hints.ts',
	'task_args.ts',
	'sveltekit_helpers.ts',
	'package_json.ts',
	// the default config, a root, reached when loading the config
	'gro_config.ts',
	// the Node module graph, the task hints, and `package_json.ts`
	'invoke_task.ts'
];

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
			let target: string;
			try {
				target = specifier.startsWith('.')
					? resolve(dirname(id), specifier)
					: createRequire(id).resolve(specifier);
			} catch {
				// a package `createRequire` can't resolve, like a lazy import's optional dependency
				violations.push(`unresolved ${specifier} <- ${to_chain(id)}`);
				continue;
			}
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

/**
 * The modules `entry` reaches by relative value imports, `entry` included, relative to `src/lib`.
 */
const to_lib_closure = (entry: string): Array<string> => {
	const reached: Set<string> = new Set([entry]);
	const queue = [entry];
	for (let id = queue.shift(); id !== undefined; id = queue.shift()) {
		for (const specifier of to_specifiers(id)) {
			if (!specifier.startsWith('.')) continue;
			const target = resolve(dirname(id), specifier);
			if (!reached.has(target)) {
				reached.add(target);
				queue.push(target);
			}
		}
	}
	return [...reached].map((id) => relative(paths.lib, id)).sort();
};

describe("the CLI entry's closure", () => {
	const host_modules = [...HOST_MODULES].map((id) => relative(paths.lib, id));
	const lists: ReadonlyArray<readonly [string, ReadonlyArray<string>]> = [
		['PACKAGE_MODULES', PACKAGE_MODULES],
		['ROOTS', ROOTS],
		['host modules', host_modules],
		['ERROR_MODULES', ERROR_MODULES],
		['NODE_ONLY', NODE_ONLY],
		['EXCEPTIONS', EXCEPTIONS]
	];

	test('every module `invoke.ts` reaches is sorted into exactly one list', () => {
		const unsorted: Array<string> = [];
		const doubled: Array<string> = [];
		for (const name of to_lib_closure(join(paths.lib, 'invoke.ts'))) {
			const holders = lists.filter(([, list]) => list.includes(name)).map(([label]) => label);
			if (holders.length === 0) unsorted.push(name);
			else if (holders.length > 1) doubled.push(`${name}: ${holders.join(', ')}`);
		}
		assert.deepEqual(unsorted, [], 'reached but in no list');
		assert.deepEqual(doubled, [], 'in more than one list');
	});

	test('each listed module is one `invoke.ts` reaches', () => {
		const reached = new Set(to_lib_closure(join(paths.lib, 'invoke.ts')));
		const listed = [...PACKAGE_MODULES, ...ROOTS, ...ERROR_MODULES, ...NODE_ONLY, ...EXCEPTIONS];
		assert.deepEqual(
			listed.filter((name) => !PACKAGE_MODULES_BEYOND_THE_CLI.includes(name) && !reached.has(name)),
			[],
			'listed but not reached'
		);
	});

	for (const name of EXCEPTIONS) {
		test(`${name} still reaches Node, or it joins \`PACKAGE_MODULES\``, () => {
			assert.notDeepEqual(to_closure_violations(join(paths.lib, name)), []);
		});
	}
});
