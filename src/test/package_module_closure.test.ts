import { describe, test, assert } from 'vitest';
import { readdirSync, readFileSync } from 'node:fs';
import { createRequire, isBuiltin } from 'node:module';
import { dirname, join, relative, resolve } from 'node:path';

import { parse_imports } from '#lib/internal/parse_imports.ts';
import { to_lib_url, use_fixture_runner } from './host_fixture_helpers.ts';
import { GRO_BUILTIN_TASK_GRANTS } from '#lib/gro_builtin_tasks.ts';
import { paths } from './test_helpers.ts';

/*

Library modules a project imports as a package on the runtime, where a package holds no grant
and `src/lib` reaches its host modules by relative path, so they'd load the Node files.
Each one's static import closure, `import type` erased, must hold no `node:` builtin
and no host module's Node file, beside the grant-free error modules,
and loading it must read nothing of the process.
A module freed for the runtime joins `PACKAGE_MODULES`.

Each builtin is an entry of its own, imported by name from `GRO_BUILTIN_TASK_IMPORTS`,
so no closure here follows that table into the builtins it imports:
a builtin with grants is in `BUILTIN_TASKS`, and one with none in `NODE_BUILTIN_TASKS`.

A builtin task takes what it reaches from its `host`, so the root's handles are all it holds:
the closure of each one in `BUILTIN_TASKS` reaches no `node:` builtin and no granted
host module, neither Node's project loader nor its module graph,
and no package outside `BUILTIN_TASK_PACKAGES`; loading it reads nothing of the process.
The grant-free host modules, the ones `fgro` builds with no grant, lend no authority,
so a builtin may reach them.
The closure is the static and dynamic `import`s the import parse sees, so a CommonJS file's
`require` is invisible to it: a package is listed with what of it the parse can't see.

Every module the CLI's entry, `invoke.ts`, reaches by relative import is one of those,
or a root, a host module, Node-only machinery, or an exception, each listed below,
so a module newly reached fails until it's sorted, and an exception that no longer
reaches Node fails until it moves to `PACKAGE_MODULES`.

*/

const PACKAGE_MODULES = [
	'builtin_host.ts',
	'cli.ts',
	'constants.ts',
	'error_detail.ts',
	'fs_paths.ts',
	'fs_search.ts',
	'gen.ts',
	'git.ts',
	'github.ts',
	'gro_builtin_task_imports.ts',
	'gro_builtin_tasks.ts',
	'gro_plugin_sveltekit_app.ts',
	'input_path.ts',
	'internal/code_points.ts',
	'internal/env_name.ts',
	'internal/error_class.ts',
	'internal/error_report.ts',
	'internal/forwarded_info_flags.ts',
	'internal/fs_boundary.ts',
	'internal/fs_handles.ts',
	'internal/posix_path.ts',
	'internal/records.ts',
	'internal/task_hints.ts',
	'invoke_task.ts',
	'modules.ts',
	'package_json.ts',
	'paths.ts',
	'run_task.ts',
	// its lazy import of `typescript`, a CommonJS package the runtime refuses, which the parse sees
	// and `createRequire` resolves, is called only by `library_load`, which no check command reaches
	'sveltekit_helpers.ts',
	'task.ts',
	'task_args.ts',
	'task_error.ts',
	'task_logging.ts',
	'task_registry.ts'
];

// the package modules that read the process when they load, each with what it reads,
// so the test of each fails once it reads nothing
const PACKAGE_MODULE_LOAD_READS: ReadonlyMap<string, ReadonlyArray<string>> = new Map([
	// fuz_util's `log.ts`, imported for `Logger` as a value, reads `PUBLIC_LOG_LEVEL` and `VITEST`
	// from a `process` global when it loads, which the runtime has none of
	['invoke_task.ts', ['env', 'process']]
]);

// the builtin tasks that take their handles from their `host`
const BUILTIN_TASKS = [
	'check.task.ts',
	'clean.task.ts',
	'format.task.ts',
	'gen.task.ts',
	'lint.task.ts',
	'release.task.ts',
	'sync.task.ts',
	'test.task.ts',
	'typecheck.task.ts'
];

// the builtin tasks with no grants, which import their host modules, so they run only on Node
const NODE_BUILTIN_TASKS = [
	'build.task.ts',
	'changeset.task.ts',
	'commit.task.ts',
	'deploy.task.ts',
	'dev.task.ts',
	'publish.task.ts',
	'reinstall.task.ts',
	'run.task.ts',
	'upgrade.task.ts'
];

// the table of the builtins' literal imports, whose imports no closure here follows
const BUILTIN_TASK_IMPORTS_ID = join(paths.lib, 'gro_builtin_task_imports.ts');

// the package modules the CLI's entry doesn't reach, loaded by tasks and genfiles
const PACKAGE_MODULES_BEYOND_THE_CLI = ['git.ts'];

// the composition roots, which import the host modules and pass what they open down
const ROOTS = ['invoke.ts'];

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
	// the Node Gro's platform, which the runtime's entry hands in instead
	'internal/node_platform.ts',
	// the Svelte config, read through Vite, a package on `node:*`, which the runtime hands in
	'svelte_config.ts',
	// the Node module graph and the Filer family it imports when first used
	'module_graph_node.ts',
	'internal/filer.ts',
	'internal/watch_dir.ts',
	'internal/parse_imports.ts',
	'resolve_specifier.ts'
];

// library modules whose closure still reaches Node, each with what it reaches
const EXCEPTIONS = [
	// the tsv host module, whose Node file a relative import loads
	'dist_rewrite_imports.ts',
	'gen_helpers.ts',
	// and through them the default config and its plugins
	'gro.config.default.ts',
	'gro_plugin_gen.ts',
	'gro_plugin_sveltekit_library.ts',
	// the config, which hashes with the blake3 host module and loads the default config
	'gro_config.ts',
	// the CLI's body, which loads the config through `gro_config.ts`
	'gro_main.ts'
];

// the host modules, whose Node files a relative import from `src/lib` loads, granted or not;
// their error modules aren't here, grant-free classes a package module may import
// the host modules that carry a grant on `fgro`, or will
const GRANTED_HOST_MODULES = [
	'fs.ts',
	'spawn.ts',
	'env.ts',
	'args.ts',
	'exit.ts',
	'stdin.ts',
	'stdout.ts',
	'stderr.ts',
	'net.ts',
	'watch.ts'
].map((name) => join(paths.lib, name));
// the host modules `fgro` builds with no grant, the rows the resolver's table marks `None`
const GRANT_FREE_HOST_MODULES = [
	'cwd.ts',
	'pid.ts',
	'runtime_version.ts',
	'blake3.ts',
	'tsv.ts'
].map((name) => join(paths.lib, name));
const HOST_MODULES: ReadonlySet<string> = new Set([
	...GRANTED_HOST_MODULES,
	...GRANT_FREE_HOST_MODULES
]);

// what a builtin task reaches only through its `host`: the granted host modules,
// and Node's project loader and module graph, which the root hands down
const BUILTIN_TASK_FORBIDDEN: ReadonlySet<string> = new Set([
	...GRANTED_HOST_MODULES,
	...['internal/node_import_module.ts', 'module_graph_node.ts', 'loader.ts', 'register.ts'].map(
		(name) => join(paths.lib, name)
	)
]);

// the packages a builtin's closure may reach, each with why and what of it the parse can't see
const BUILTIN_TASK_PACKAGES: ReadonlyMap<string, string> = new Map([
	['@fuzdev/fuz_util', 'ES modules, every import parsed'],
	['@fuzdev/fuz_schema', 'ES modules, every import parsed'],
	[
		'typescript',
		"CommonJS, so its `require`s, `fs` and `child_process` among them, aren't seen: " +
			"`sveltekit_helpers.ts` imports it lazily to parse a tsconfig's text"
	]
]);

/**
 * The package name of a file under `node_modules`, or `null` for any other file.
 */
const to_package_name = (id: string): string | null => {
	const at = id.lastIndexOf('/node_modules/');
	if (at === -1) return null;
	const segments = id.slice(at + '/node_modules/'.length).split('/');
	return segments[0]!.startsWith('@') ? segments[0] + '/' + segments[1] : segments[0]!;
};

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
		if (id === BUILTIN_TASK_IMPORTS_ID) continue;
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

/**
 * What the static import closure of the builtin task at `entry` reaches that it can't,
 * each with the chain from `entry` that reaches it: a `node:` builtin,
 * a module in `BUILTIN_TASK_FORBIDDEN`, or a package outside `BUILTIN_TASK_PACKAGES`.
 */
const to_builtin_task_violations = (entry: string): Array<string> => {
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
		if (id === BUILTIN_TASK_IMPORTS_ID) continue;
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
				continue; // a package `createRequire` can't resolve, like a lazy import's optional dependency
			}
			if (BUILTIN_TASK_FORBIDDEN.has(target)) {
				violations.push(`${relative(paths.root, target)} <- ${to_chain(id)}`);
				continue;
			}
			// a builtin module on the runtime, which replaces its Node file rather than loading it
			if (GRANT_FREE_HOST_MODULES.includes(target)) continue;
			if (!parents.has(target)) {
				const package_name = to_package_name(target);
				if (package_name !== null && !BUILTIN_TASK_PACKAGES.has(package_name)) {
					violations.push(`package ${package_name} <- ${to_chain(id)}`);
					continue;
				}
				parents.set(target, id);
				queue.push(target);
			}
		}
	}
	return violations;
};

const { run_fixture_json } = use_fixture_runner('');

/**
 * What of the global `process` loading the module `name` in `src/lib` reads,
 * the modules `preloaded` loaded first, unwatched.
 */
const to_load_reads = (name: string, preloaded: ReadonlyArray<string> = []): unknown =>
	run_fixture_json(
		`for (const url of ${JSON.stringify(preloaded.map((id) => to_lib_url(relative(paths.lib, id))))}) {
			await import(url);
		 }
		 const real = globalThis.process;
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

describe('library modules a package imports on the runtime', () => {
	for (const name of PACKAGE_MODULES) {
		test(`${name} reaches no \`node:\` builtin or host module`, () => {
			assert.deepEqual(to_closure_violations(join(paths.lib, name)), []);
		});

		const reads = PACKAGE_MODULE_LOAD_READS.get(name) ?? [];
		test(`${name} reads ${reads.length ? reads.join(', ') : 'nothing'} of the process when it loads`, () => {
			assert.deepEqual(to_load_reads(name), reads);
		});
	}
});

describe('builtin tasks that take their handles from their `host`', () => {
	for (const name of BUILTIN_TASKS) {
		test(`${name} reaches no granted host module, project loader, module graph, or Node authority`, () => {
			assert.deepEqual(to_builtin_task_violations(join(paths.lib, name)), []);
		});

		// the grant-free host modules are builtins on the runtime, built before any module loads,
		// so what their Node files read when they load is theirs
		test(`${name} reads nothing of the process when it loads`, () => {
			assert.deepEqual(to_load_reads(name, GRANT_FREE_HOST_MODULES), []);
		});
	}
});

/**
 * The chains by which the static import closure of the module at `entry` reaches a module
 * in `BUILTIN_TASK_FORBIDDEN`: a granted host module, Node's project loader, or its module graph.
 */
const to_forbidden_reaches = (entry: string): Array<string> => {
	const parents: Map<string, string | null> = new Map([[entry, null]]);
	const queue = [entry];
	const reaches: Array<string> = [];
	const to_chain = (id: string): string => {
		const chain: Array<string> = [];
		for (let at: string | null = id; at !== null; at = parents.get(at) ?? null) {
			chain.push(relative(paths.root, at));
		}
		return chain.join(' <- ');
	};
	for (let id = queue.shift(); id !== undefined; id = queue.shift()) {
		if (id === BUILTIN_TASK_IMPORTS_ID) continue;
		for (const specifier of to_specifiers(id)) {
			// a package's imports are its own, and gro's reach its host modules only by path
			if (!specifier.startsWith('.')) continue;
			const target = resolve(dirname(id), specifier);
			if (BUILTIN_TASK_FORBIDDEN.has(target)) {
				reaches.push(`${relative(paths.root, target)} <- ${to_chain(id)}`);
				continue;
			}
			if (!parents.has(target)) {
				parents.set(target, id);
				queue.push(target);
			}
		}
	}
	return reaches;
};

// the default config and every plugin Gro ships, which a project's config loads,
// and the config loads on every command, so what they import when they load links then;
// a plugin takes its handles from the `host` of the context it's set up with
const CONFIG_AND_PLUGINS = [
	'gro.config.default.ts',
	...readdirSync(paths.lib).filter((name) => name.startsWith('gro_plugin_') && name.endsWith('.ts'))
];

describe('the default config and the plugins Gro ships', () => {
	test('include every plugin module', () => {
		assert.includeMembers(CONFIG_AND_PLUGINS, [
			'gro_plugin_gen.ts',
			'gro_plugin_server.ts',
			'gro_plugin_sveltekit_app.ts',
			'gro_plugin_sveltekit_library.ts'
		]);
	});

	// the walk finds a reach when there is one, directly and through another module
	test('finds what a root reaches', () => {
		const reaches = to_forbidden_reaches(join(paths.lib, 'invoke.ts'));
		assert.include(reaches, 'src/lib/fs.ts <- src/lib/invoke.ts');
		assert.include(
			reaches,
			'src/lib/module_graph_node.ts <- src/lib/internal/node_platform.ts <- src/lib/invoke.ts'
		);
	});

	for (const name of CONFIG_AND_PLUGINS) {
		test(`${name} imports no granted host module, project loader, or module graph when it loads`, () => {
			assert.deepEqual(to_forbidden_reaches(join(paths.lib, name)), []);
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
		if (id === BUILTIN_TASK_IMPORTS_ID) continue;
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

test('every builtin is in `BUILTIN_TASKS` or `NODE_BUILTIN_TASKS`, by whether it has grants', () => {
	const granted = Object.entries(GRO_BUILTIN_TASK_GRANTS)
		.filter(([, grants]) => grants !== null)
		.map(([name]) => name + '.task.ts');
	const ungranted = Object.entries(GRO_BUILTIN_TASK_GRANTS)
		.filter(([, grants]) => grants === null)
		.map(([name]) => name + '.task.ts');
	assert.deepEqual([...BUILTIN_TASKS].sort(), granted.sort());
	assert.deepEqual([...NODE_BUILTIN_TASKS].sort(), ungranted.sort());
});

test('the table imports each builtin by a literal import of its module, and nothing else', () => {
	assert.deepEqual(
		to_specifiers(BUILTIN_TASK_IMPORTS_ID),
		Object.keys(GRO_BUILTIN_TASK_GRANTS).map((name) => `./${name}.task.ts`)
	);
});

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

	// what the binary's entry calls, where a relative import of a host module is its builtin:
	// it reaches no `node:` builtin and only the grant-free host modules
	test('the CLI body, `gro_main.ts`, reaches Node only through the grant-free host modules', () => {
		const grant_free = GRANT_FREE_HOST_MODULES.map((id) => relative(paths.root, id) + ' <- ');
		const violations = to_closure_violations(join(paths.lib, 'gro_main.ts'));
		assert.ok(violations.length > 0);
		assert.deepEqual(
			violations.filter((violation) => !grant_free.some((prefix) => violation.startsWith(prefix))),
			[]
		);
	});

	for (const name of EXCEPTIONS) {
		test(`${name} still reaches Node, or it joins \`PACKAGE_MODULES\``, () => {
			assert.notDeepEqual(to_closure_violations(join(paths.lib, name)), []);
		});
	}
});
