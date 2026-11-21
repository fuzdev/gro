import { describe, test, assert } from 'vitest';
import { readdirSync, readFileSync } from 'node:fs';
import { createRequire, isBuiltin } from 'node:module';
import { dirname, join, relative, resolve } from 'node:path';
import * as s from '@fuzdev/fuz_schema/schema.ts';

import { parse_imports_typescript } from '#lib/tsv.ts';
import { to_lib_url, use_fixture_runner } from './host_fixture_helpers.ts';
import { GRO_BUILTIN_TASK_GRANTS } from '#lib/gro_builtin_tasks.ts';
import { paths } from './test_helpers.ts';

/*

The embed is the binary's copy of Gro's own `src/lib`, and `src/embed.json` sorts every file there:
the modules it takes, and the ones it leaves out, by group, each group with its reason.
A new file fails until it's sorted.
An embedded module's relative import of a host module or its error module is the runtime's builtin,
and the embed's origin holds no grant, so each embedded module's static import closure,
`import type` erased, holds no `node:` builtin, no granted host module, no file the embed leaves out,
and no package outside `PACKAGE_MODULE_PACKAGES`, and loading it reads nothing of the process
but what `EMBED_LOAD_READS` lists. A module in the embed's `reaches_node` group fails
once it reaches none of those, until the embed takes it.

Library modules a project imports as a package on the runtime, where a package holds no grant
and `src/lib` reaches its host modules by relative path, so they'd load the Node files.
Each one's static import closure, `import type` erased, must hold no `node:` builtin
and no host module's Node file, beside the grant-free error modules.
A module freed for the runtime joins `PACKAGE_MODULES`.

Each builtin is an entry of its own, imported by name from `GRO_BUILTIN_TASK_IMPORTS`,
so no closure here follows that table into the builtins it imports:
a builtin with grants is embedded, and one with none in the embed's `node_builtin_tasks` group.

A builtin task takes what it reaches from its `host`, so the root's handles are all it holds:
the closure of each one in `BUILTIN_TASKS` reaches no `node:` builtin and no granted
host module, neither Node's project loader nor its module graph,
and no package outside `BUILTIN_TASK_PACKAGES`.
The grant-free host modules, the ones `fgro` builds with no grant, lend no authority,
so a builtin may reach them.
The closure is the static and dynamic `import`s the import parse sees, so a CommonJS file's
`require` is invisible to it: a package is listed with what of it the parse can't see.

Every module the CLI's entry, `invoke.ts`, reaches by relative import is one of those,
an exception, or in one of the embed's excluded groups,
so a module newly reached fails until it's sorted, and an exception that no longer
reaches Node fails until it moves to `PACKAGE_MODULES`.

*/

/**
 * A group of the files the embed leaves out.
 */
const EmbedGroup = s.object({ reason: s.string, modules: s.array(s.string) });

/**
 * The embed's sort of `src/lib`, each path relative to it.
 */
const EmbedSort = s.object({
	description: s.string,
	embedded: s.array(s.string),
	excluded: s.object({
		host_modules: EmbedGroup,
		node_roots: EmbedGroup,
		node_loader: EmbedGroup,
		node_only: EmbedGroup,
		node_builtin_tasks: EmbedGroup,
		genfiles: EmbedGroup,
		reaches_node: EmbedGroup
	})
});

const EMBED = s.parse_or_throw(
	EmbedSort,
	JSON.parse(readFileSync(join(paths.root, 'src/embed.json'), 'utf8'))
);
const EMBEDDED: ReadonlySet<string> = new Set(EMBED.embedded);

const PACKAGE_MODULES = [
	'builtin_host.ts',
	'cli.ts',
	'constants.ts',
	'env_files.ts',
	'error_detail.ts',
	'fs_paths.ts',
	'fs_search.ts',
	'gen.ts',
	'git.ts',
	'github.ts',
	'gro_builtin_task_imports.ts',
	'gro_builtin_tasks.ts',
	'gro_plugin_server.ts',
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

// the embedded modules that read the process when they load, each with what it reads,
// so the test of each fails once it reads nothing
const EMBED_LOAD_READS: ReadonlyMap<string, ReadonlyArray<string>> = new Map([
	// fuz_util's `log.ts`, imported for `Logger` as a value, reads `PUBLIC_LOG_LEVEL` and `VITEST`
	// from a `process` global when it loads, which the runtime has none of
	['invoke_task.ts', ['env', 'process']],
	// and through `invoke_task.ts`, the CLI's body
	['gro_main.ts', ['env', 'process']]
]);

// the builtin tasks that take their handles from their `host`, the ones with grants
const BUILTIN_TASKS = Object.entries(GRO_BUILTIN_TASK_GRANTS)
	.filter(([, grants]) => grants !== null)
	.map(([name]) => name + '.task.ts');

// the table of the builtins' literal imports, whose imports no closure here follows
const BUILTIN_TASK_IMPORTS_ID = join(paths.lib, 'gro_builtin_task_imports.ts');

// the package modules the CLI's entry doesn't reach, loaded by tasks and genfiles
const PACKAGE_MODULES_BEYOND_THE_CLI = ['env_files.ts', 'git.ts', 'gro_plugin_server.ts'];

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

// the default config and every plugin Gro ships, which a project's config loads,
// and the config loads on every command, so what they import when they load links then;
// a plugin takes its handles from the `host` of the context it's set up with
const CONFIG_AND_PLUGINS = [
	'gro.config.default.ts',
	...readdirSync(paths.lib).filter((name) => name.startsWith('gro_plugin_') && name.endsWith('.ts'))
];

// the grant-free error modules beside the host modules, which a package module may import,
// rows of the resolver's table too
const ERROR_MODULES = [
	'env_error.ts',
	'fs_error.ts',
	'net_error.ts',
	'spawn_error.ts',
	'stdio_error.ts'
].map((name) => join(paths.lib, name));
// the host modules `fgro` builds with no grant, the rows the resolver's table marks `None`
const GRANT_FREE_HOST_MODULES = [
	'cwd.ts',
	'pid.ts',
	'runtime_version.ts',
	'blake3.ts',
	'tsv.ts'
].map((name) => join(paths.lib, name));
// the host modules that carry a grant on `fgro`, or will
const GRANTED_HOST_MODULES = [
	'args.ts',
	'env.ts',
	'exit.ts',
	'fs.ts',
	'net.ts',
	'spawn.ts',
	'stderr.ts',
	'stdin.ts',
	'stdout.ts',
	'watch.ts'
].map((name) => join(paths.lib, name));
// the host modules, whose Node files a relative import from `src/lib` loads, granted or not;
// their error modules aren't here, grant-free classes a package module may import
const HOST_MODULES: ReadonlySet<string> = new Set([
	...GRANTED_HOST_MODULES,
	...GRANT_FREE_HOST_MODULES
]);
// the builtins an embedded module links, since its origin holds no grant
const EMBED_BUILTINS: ReadonlySet<string> = new Set([...GRANT_FREE_HOST_MODULES, ...ERROR_MODULES]);

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

// the packages a package module's closure may reach: a CommonJS package's `require`s
// aren't parsed, so a package is admitted by name, never by what the walk sees of it
const PACKAGE_MODULE_PACKAGES: ReadonlyMap<string, string> = new Map([
	...BUILTIN_TASK_PACKAGES,
	['esm-env', 'ES modules, every import parsed: `@fuzdev/fuz_util/log.ts` reads it']
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
 * The specifiers of the JS or TS module at `id` that load a module, failing on a parse error:
 * every request but an `import type` or `export type … from`, so an `import {type T}`
 * counts, kept by the type strip as an import whose side effects run (`verbatimModuleSyntax`).
 */
const to_specifiers = (id: string): Array<string> => {
	const parsed = parse_imports_typescript(readFileSync(id, 'utf8'));
	if (!parsed.ok) assert.fail(`${id} doesn't parse: ${parsed.error.message}`);
	const specifiers: Array<string> = [];
	for (const record of parsed.imports) {
		if (record.typeOnly === 'statement') continue;
		const value = record.specifier?.value;
		if (value) specifiers.push(value);
	}
	return specifiers;
};

/**
 * What a closure may reach beside packages and the modules it follows.
 */
interface ClosureRules {
	/**
	 * The modules a relative import links as the runtime's builtins, loading no file.
	 */
	linked: ReadonlySet<string>;
	/**
	 * Whether the closure may load the module at `id`, a file outside a package.
	 */
	admits: (id: string) => boolean;
}

// a package module's, whose relative imports load the host modules' Node files
const PACKAGE_MODULE_RULES: ClosureRules = {
	linked: new Set(),
	admits: (id) => !HOST_MODULES.has(id)
};

// an embedded module's, whose relative imports of the grant-free builtins link them,
// and which loads no file outside a package but the embed's
const EMBED_RULES: ClosureRules = {
	linked: EMBED_BUILTINS,
	admits: (id) => EMBEDDED.has(relative(paths.lib, id))
};

/**
 * What the static import closure of the module at `entry` reaches that `rules` refuse,
 * each with the chain from `entry` that reaches it: a `node:` builtin, a module `rules` don't admit,
 * or a package outside `PACKAGE_MODULE_PACKAGES`.
 */
const to_closure_violations = (entry: string, rules: ClosureRules): Array<string> => {
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
			if (rules.linked.has(target)) continue;
			const package_name = to_package_name(target);
			if (package_name === null && !rules.admits(target)) {
				violations.push(`${relative(paths.root, target)} <- ${to_chain(id)}`);
				continue;
			}
			if (!parents.has(target)) {
				if (package_name !== null && !PACKAGE_MODULE_PACKAGES.has(package_name)) {
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

/**
 * Every file of `src/lib`, relative to it.
 */
const to_lib_files = (): Array<string> =>
	readdirSync(paths.lib, { recursive: true, withFileTypes: true })
		.filter((dirent) => dirent.isFile())
		.map((dirent) => relative(paths.lib, join(dirent.parentPath, dirent.name)))
		.sort();

describe('the embed', () => {
	const lists: ReadonlyArray<readonly [string, ReadonlyArray<string>]> = [
		['embedded', EMBED.embedded],
		...Object.entries(EMBED.excluded).map(
			([group, { modules }]) => [group, modules] as [string, ReadonlyArray<string>]
		)
	];

	test('sorts every file of `src/lib` into exactly one list, and lists only those', () => {
		const files = to_lib_files();
		const unsorted: Array<string> = [];
		const doubled: Array<string> = [];
		for (const name of files) {
			const holders = lists.filter(([, list]) => list.includes(name)).map(([label]) => label);
			if (holders.length === 0) unsorted.push(name);
			else if (holders.length > 1) doubled.push(`${name}: ${holders.join(', ')}`);
		}
		assert.deepEqual(unsorted, [], 'in no list');
		assert.deepEqual(doubled, [], 'in more than one list');
		const listed = lists.flatMap(([, list]) => list);
		assert.deepEqual(
			listed.filter((name) => !files.includes(name)),
			[],
			'listed but not in `src/lib`'
		);
		assert.strictEqual(listed.length, files.length, 'listed twice in one list');
	});

	test('keeps each list sorted', () => {
		for (const [label, list] of lists) {
			assert.deepEqual(list, [...list].sort(), label);
		}
	});

	test('excludes the builtins with no grants, and embeds the rest', () => {
		const ungranted = Object.entries(GRO_BUILTIN_TASK_GRANTS)
			.filter(([, grants]) => grants === null)
			.map(([name]) => name + '.task.ts');
		assert.deepEqual(EMBED.excluded.node_builtin_tasks.modules, ungranted.sort());
		assert.includeMembers(EMBED.embedded, BUILTIN_TASKS);
	});

	test('embeds the package modules, the exceptions, the default config, and the plugins', () => {
		assert.includeMembers(EMBED.embedded, [
			...PACKAGE_MODULES,
			...EXCEPTIONS,
			...CONFIG_AND_PLUGINS
		]);
	});

	test('excludes the host modules and their error modules, and nothing else as one', () => {
		assert.deepEqual(
			EMBED.excluded.host_modules.modules,
			[...GRANTED_HOST_MODULES, ...GRANT_FREE_HOST_MODULES, ...ERROR_MODULES]
				.map((id) => relative(paths.lib, id))
				.sort()
		);
	});

	// the walk finds each kind of reach: a `node:` builtin, a granted host module,
	// a file the embed leaves out, and a package beyond a package module's
	test('finds what a root and an excluded module reach', () => {
		const violations = to_closure_violations(join(paths.lib, 'invoke.ts'), EMBED_RULES);
		assert.include(violations, 'node:util <- src/lib/invoke.ts');
		assert.include(violations, 'src/lib/fs.ts <- src/lib/invoke.ts');
		assert.include(violations, 'src/lib/internal/node_platform.ts <- src/lib/invoke.ts');
		assert.include(
			to_closure_violations(join(paths.lib, 'library_load.ts'), EMBED_RULES),
			'package svelte-docinfo <- src/lib/library_load.ts'
		);
	});

	for (const name of EMBED.embedded) {
		test(`${name} reaches no \`node:\` builtin, granted host module, or file the embed leaves out`, () => {
			assert.deepEqual(to_closure_violations(join(paths.lib, name), EMBED_RULES), []);
		});

		// the grant-free host modules and their error modules are builtins on the runtime,
		// built before any module loads, so what their Node files read when they load is theirs
		const reads = EMBED_LOAD_READS.get(name) ?? [];
		test(`${name} reads ${reads.length ? reads.join(', ') : 'nothing'} of the process when it loads`, () => {
			assert.deepEqual(to_load_reads(name, [...EMBED_BUILTINS]), reads);
		});
	}

	for (const name of EMBED.excluded.reaches_node.modules) {
		test(`${name} still reaches Node, or the embed takes it`, () => {
			assert.notDeepEqual(to_closure_violations(join(paths.lib, name), EMBED_RULES), []);
		});
	}
});

describe('library modules a package imports on the runtime', () => {
	for (const name of PACKAGE_MODULES) {
		test(`${name} reaches no \`node:\` builtin or host module`, () => {
			assert.deepEqual(to_closure_violations(join(paths.lib, name), PACKAGE_MODULE_RULES), []);
		});
	}
});

describe('builtin tasks that take their handles from their `host`', () => {
	for (const name of BUILTIN_TASKS) {
		test(`${name} reaches no granted host module, project loader, module graph, or Node authority`, () => {
			assert.deepEqual(to_builtin_task_violations(join(paths.lib, name)), []);
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

		// as a builtin's, so a project's config that composes it links where `node:` doesn't
		test(`${name} reaches no \`node:\` builtin or package beyond a builtin's when it loads`, () => {
			assert.deepEqual(to_builtin_task_violations(join(paths.lib, name)), []);
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

// the genfile that writes the table declares the CLI's row, and nothing it loads imports the row,
// so a lost or renamed row is regenerated rather than repaired by hand
test("the summaries genfile loads nothing that imports the CLI's row it renders", () => {
	const closure = to_lib_closure(join(paths.lib, 'gro_builtin_tasks.gen.ts'));
	assert.include(closure, 'task_registry.ts');
	assert.notInclude(closure, 'gro_main.ts');
	const importers = closure.filter((name) =>
		/import\s*(type\s*)?\{[^}]*\bGRO_CLI_GRANTS\b[^}]*\}\s*from/.test(
			readFileSync(join(paths.lib, name), 'utf8')
		)
	);
	assert.deepEqual(importers, []);
	assert.match(
		readFileSync(join(paths.lib, 'gro_main.ts'), 'utf8'),
		/import\s*\{[^}]*\bGRO_CLI_GRANTS\b[^}]*\}\s*from '\.\/gro_builtin_tasks\.ts'/
	);
});

test('the table imports each builtin by a literal import of its module, and nothing else', () => {
	assert.deepEqual(
		to_specifiers(BUILTIN_TASK_IMPORTS_ID),
		Object.keys(GRO_BUILTIN_TASK_GRANTS).map((name) => `./${name}.task.ts`)
	);
});

describe("the CLI entry's closure", () => {
	const lists: ReadonlyArray<readonly [string, ReadonlyArray<string>]> = [
		['PACKAGE_MODULES', PACKAGE_MODULES],
		['EXCEPTIONS', EXCEPTIONS],
		...Object.entries(EMBED.excluded).map(
			([group, { modules }]) => [`the embed's ${group}`, modules] as [string, ReadonlyArray<string>]
		)
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
		const listed = [...PACKAGE_MODULES, ...EXCEPTIONS];
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
		const violations = to_closure_violations(join(paths.lib, 'gro_main.ts'), PACKAGE_MODULE_RULES);
		assert.ok(violations.length > 0);
		assert.deepEqual(
			violations.filter((violation) => !grant_free.some((prefix) => violation.startsWith(prefix))),
			[]
		);
	});

	for (const name of EXCEPTIONS) {
		test(`${name} still reaches Node, or it joins \`PACKAGE_MODULES\``, () => {
			assert.notDeepEqual(to_closure_violations(join(paths.lib, name), PACKAGE_MODULE_RULES), []);
		});
	}
});
