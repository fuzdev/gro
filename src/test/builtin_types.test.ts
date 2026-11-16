import { describe, test, assert } from 'vitest';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import ts from 'typescript';

import { paths } from '#lib/paths.ts';

import {
	BUILTIN_DIR,
	read_builtin_rows,
	read_implementation,
	read_implementation_specifiers,
	type BuiltinRow
} from './builtin_test_helpers.ts';

/*

A builtin module of the runtime declares no type of its own: its exports have one declaration,
the Node host module's (`src/lib/<name>.ts`). Its implementation
(`crates/gro_runtime/js/builtin/<specifier>`) is a script, one function in two levels: the
outer takes the bootstrap object and returns the inner, a function of its grant's scope that
returns the namespace. This checks, for every row of the runtime's builtin table, the namespace
the inner level infers against the Node module's exports, exactly and both ways, and each
level's parameters, in a program with the implementation's own declaration of the bootstrap
object, the ES library, no Node types, and the Node module as its emitted declarations, which
name no Node type (`host_module_declarations.test.ts`).

The implementation's own program has no library at all (`builtin_intrinsics.test.ts`), so the
Node module's types couldn't be written there; the function's types are the same in both.

The rows are read from the table's source, never listed here, so a row can't join the table
without its Node module being compared, and the implementations on disk are exactly the rows'.

*/

const JS_DIR = dirname(BUILTIN_DIR);

/**
 * The implementation as a module, so its one function can be named: a script exports nothing.
 */
const to_subject = (implementation: string): string =>
	`export const implementation =\n${implementation}`;

/**
 * The check, a module beside the implementation: the namespace's exports are the Node module's
 * and `default`, each export exactly the Node module's, the two namespaces assign both ways,
 * the outer level takes the bootstrap object alone, and the inner level it returns takes one
 * argument, its grant's scope, which no row's grant has.
 */
const to_check = ({ name }: BuiltinRow): string => `
import type { implementation } from './${name}.subject.ts';
import type * as node from './${name}.node_declarations.d.ts';

type Equal<A, B> = (<T>() => T extends A ? 1 : 2) extends <T>() => T extends B ? 1 : 2 ? true : false;
// a class by its public shape, its constructor's parameters and its instances' members: the
// host makes a builtin's class, so its type is a claim, which no class type equals
type Shape<T> = T extends abstract new (...args: infer P) => infer I ? [P, { [K in keyof I]: I[K] }] : T;
type Inner = ReturnType<typeof implementation>;
type Namespace = ReturnType<Inner>;
type Builtin = Omit<Namespace, 'default'>;
type Node = typeof node;

export const exports: Equal<keyof Namespace, keyof Node | 'default'> = true;
export const no_default: Equal<Namespace['default'], undefined> = true;
export const members: Equal<{ [K in keyof Node]: Equal<Shape<Builtin[K]>, Shape<Node[K]>> }[keyof Node], true> = true;
export const to_node = (builtin: Builtin): Node => builtin;
export const to_builtin = (node: Node): Builtin => node;
export const outer_parameters: Equal<Parameters<typeof implementation>, [bootstrap: Bootstrap]> = true;
export const inner_parameters: Equal<Parameters<Inner>, [_scope: undefined]> = true;
`;

// the files every check reads from disk, parsed once
const source_files: Map<string, ts.SourceFile | undefined> = new Map();

/**
 * Type-checks a row's check beside its implementation, against `declarations` standing in for
 * the Node module's, under the bootstrap's tsconfig, the ES library with no other types.
 * @param row - the builtin's row
 * @param declarations - the Node module's declarations
 * @param implementation - the implementation's source in place of its file, when given
 * @returns the check's diagnostics, each as text
 */
const check_types = (
	row: BuiltinRow,
	declarations: string,
	implementation = read_implementation(row),
	bootstrap = readFileSync(join(BUILTIN_DIR, 'bootstrap.d.ts'), 'utf8')
): Array<string> => {
	const config_path = join(JS_DIR, 'tsconfig.json');
	const config = ts.readConfigFile(config_path, ts.sys.readFile);
	assert.ok(!config.error, 'the bootstrap tsconfig reads');
	const { options } = ts.parseJsonConfigFileContent(config.config, ts.sys, JS_DIR);
	const dir = dirname(row.path);
	const check_path = join(dir, `${row.name}.types_check.ts`);
	const virtual = new Map([
		[check_path, to_check(row)],
		[join(dir, `${row.name}.subject.ts`), to_subject(implementation)],
		[join(dir, `${row.name}.node_declarations.d.ts`), declarations],
		[join(BUILTIN_DIR, 'bootstrap.d.ts'), bootstrap]
	]);
	const host = ts.createCompilerHost(options);
	const get_source_file = host.getSourceFile.bind(host);
	host.getSourceFile = (file_name, language_version, ...rest) => {
		const text = virtual.get(file_name);
		if (text !== undefined) return ts.createSourceFile(file_name, text, language_version, true);
		if (!source_files.has(file_name)) {
			source_files.set(file_name, get_source_file(file_name, language_version, ...rest));
		}
		return source_files.get(file_name);
	};
	const file_exists = host.fileExists.bind(host);
	host.fileExists = (file_name) => virtual.has(file_name) || file_exists(file_name);
	const program = ts.createProgram(
		[check_path, join(BUILTIN_DIR, 'bootstrap.d.ts')],
		{ ...options, allowImportingTsExtensions: true },
		host
	);
	return ts
		.getPreEmitDiagnostics(program)
		.map((d) => ts.flattenDiagnosticMessageText(d.messageText, '\n'));
};

const node_declarations = (name: string): string => {
	const module_path = join(paths.lib, `${name}.ts`);
	const result = ts.transpileDeclaration(readFileSync(module_path, 'utf8'), {
		fileName: module_path,
		compilerOptions: { module: ts.ModuleKind.NodeNext, target: ts.ScriptTarget.ESNext }
	});
	return result.outputText;
};

const rows = read_builtin_rows();

describe('the builtin table', () => {
	test("the implementations are the table's rows", () => {
		assert.deepEqual(read_implementation_specifiers(), rows.map((row) => row.specifier).sort());
		// each kind of export the checks below mutate is among them
		for (const name of [
			'args',
			'blake3',
			'cwd',
			'env',
			'env_error',
			'exit',
			'pid',
			'runtime_version'
		]) {
			assert.ok(
				rows.some((row) => row.name === name),
				name
			);
		}
	});
});

describe.each(rows)('the $name builtin types', (row) => {
	test("the builtin's exports are exactly the Node module's", () => {
		assert.deepEqual(check_types(row, node_declarations(row.name)), []);
	});

	test('a mismatch fails the check', () => {
		const declarations = node_declarations(row.name);
		// every export of the Node module widened, one at a time
		const declared = Array.from(declarations.matchAll(/^export declare const (\w+): (.+);$/gm));
		assert.isAbove(declared.length, 0, 'the declarations to mutate are there');
		// and every class, each by its constructor's last parameter and by one more member
		const classes = Array.from(
			declarations.matchAll(
				/^export declare class (\w+)[^\n]*\{\n(?:.*\n)*?\s*(constructor\((.*)\);)\n\}$/gm
			)
		);
		// every value the module exports is among them, so none escapes the widening
		assert.lengthOf(
			Array.from(declarations.matchAll(/^export declare /gm)),
			declared.length + classes.length,
			'every export the check widens is a const declared on one line or a class'
		);
		for (const [line, name, type] of declared) {
			const widened = declarations.replace(
				line,
				`export declare const ${name}: (${type}) | undefined;`
			);
			assert.notDeepEqual(check_types(row, widened), [], `${name} widened`);
		}
		for (const [, name, constructor, parameters] of classes) {
			assert.ok(name && constructor && parameters !== undefined, 'the class to mutate is read');
			const widened = declarations.replace(
				constructor,
				`constructor(${parameters}, extra?: number);`
			);
			assert.notDeepEqual(check_types(row, widened), [], `${name}'s constructor widened`);
			const grown = declarations.replace(constructor, `extra: number;\n${constructor}`);
			assert.notDeepEqual(check_types(row, grown), [], `${name}'s instances grown`);
		}
		// an export the builtin doesn't have
		assert.notDeepEqual(
			check_types(row, `${declarations}\nexport declare const extra: string;\n`),
			[]
		);
	});

	test("a mismatch on the builtin's side fails the check", () => {
		const declarations = node_declarations(row.name);
		const source = read_implementation(row);
		const bootstrap = readFileSync(join(BUILTIN_DIR, 'bootstrap.d.ts'), 'utf8');
		// the namespace, made by the script, or by the host, as `bootstrap.d.ts` declares the op
		// that returns it: the row's own, or the one that serves every row with native members,
		// under the row's specifier
		const made = /default: undefined\s*\}/;
		const host_made =
			[`readonly op_gro_${row.name}: `, `readonly '${row.specifier}': {`].find((declared) =>
				bootstrap.includes(declared)
			) ?? `readonly op_gro_${row.name}: `;
		const by_host = !made.test(source);
		/**
		 * The check with the namespace's `default: undefined` as `to`, written as the script's
		 * literal writes a property, or as the op's declared type does, with `readonly`.
		 */
		const check_namespace = (to: string): Array<string> => {
			if (!by_host) {
				const property = to.replace(/readonly /g, '');
				return check_types(row, declarations, source.replace(made, `${property} }`));
			}
			const at = bootstrap.indexOf(host_made);
			assert.isAtLeast(at, 0, 'the op making the namespace is declared');
			const end = bootstrap.indexOf('readonly default: undefined;', at);
			assert.isAtLeast(end, 0, 'the namespace to mutate is there');
			const mutated = `${bootstrap.slice(0, end)}readonly ${to};${bootstrap.slice(end + 28)}`;
			return check_types(row, declarations, source, mutated);
		};
		// an export the Node module doesn't have
		assert.notDeepEqual(check_namespace("default: undefined, readonly extra: ''"), []);
		// a `default` that isn't `undefined`
		assert.notDeepEqual(check_namespace('default: null'), []);
		/**
		 * Asserts the check fails once `from` in the implementation is `to`.
		 */
		const assert_level_mismatch = (from: string, to: string): void => {
			assert.include(source, from, 'the level to mutate is there');
			assert.notDeepEqual(check_types(row, declarations, source.replace(from, to)), [], to);
		};
		// an outer level that takes the scope too, and one that takes another bootstrap object
		const outer = '(function (bootstrap: Bootstrap) {';
		assert_level_mismatch(outer, '(function (bootstrap: Bootstrap, _scope: undefined) {');
		assert_level_mismatch(outer, '(function (bootstrap: Bootstrap & { extra: 1 }) {');
		// an inner level that takes no scope, one that takes another, and one that takes more
		const inner = 'return (_scope: undefined) =>';
		assert_level_mismatch(inner, 'return () =>');
		assert_level_mismatch(inner, 'return (_scope: string) =>');
		assert_level_mismatch(inner, 'return (_scope: undefined, _extra: undefined) =>');
		// one level, the namespace made by the function of the bootstrap object
		assert_level_mismatch(inner, 'return');
	});
});

describe('a retyped export fails its check', () => {
	const find_row = (name: string): BuiltinRow => {
		const row = rows.find((row) => row.name === name);
		assert.ok(row, name);
		return row;
	};

	/**
	 * Asserts the check of `name` fails once `from` in the builtin's source, the declarations
	 * of its ops (`bootstrap.d.ts`), or the Node module's declarations (`side`) is `to`.
	 */
	const assert_mismatch = (
		name: string,
		side: 'builtin' | 'bootstrap' | 'node',
		from: string,
		to: string
	) => {
		const row = find_row(name);
		const declarations = node_declarations(name);
		const source = read_implementation(row);
		const bootstrap = readFileSync(join(BUILTIN_DIR, 'bootstrap.d.ts'), 'utf8');
		const text = side === 'builtin' ? source : side === 'bootstrap' ? bootstrap : declarations;
		assert.include(text, from, 'the text to mutate is there');
		const mutated = text.replace(from, to);
		const diagnostics =
			side === 'builtin'
				? check_types(row, declarations, mutated)
				: side === 'bootstrap'
					? check_types(row, declarations, source, mutated)
					: check_types(row, mutated);
		assert.notDeepEqual(diagnostics, [], `${name}: ${from} as ${to}`);
	};

	test('args', () => {
		const declared = 'export declare const args: ReadonlyArray<string>;';
		assert_mismatch('args', 'node', declared, 'export declare const args: Array<string>;');
		assert_mismatch(
			'args',
			'node',
			declared,
			'export declare const args: ReadonlyArray<string | number>;'
		);
		// the array claimed mutable
		assert_mismatch(
			'args',
			'bootstrap',
			'readonly args: ReadonlyArray<string>;',
			'readonly args: Array<string>;'
		);
	});

	test('env_error', () => {
		// the class's constructor, its instances, and its guard, each claimed by the host
		const claim =
			"readonly '@fuzdev/gro/env_error.ts': {\n\t\t\t\t\treadonly EnvError: new (\n\t\t\t\t\t\tkind: 'not_granted',";
		assert_mismatch('env_error', 'bootstrap', claim, claim.replace("'not_granted',", 'string,'));
		assert_mismatch(
			'env_error',
			'bootstrap',
			'options?: { cause?: unknown }',
			'options?: { cause?: string }'
		);
		// a message parameter, which neither class takes
		assert_mismatch(
			'env_error',
			'bootstrap',
			'target: string,\n\t\t\t\t\t\toptions?: { cause?: unknown }',
			'target: string,\n\t\t\t\t\t\tmessage?: string,\n\t\t\t\t\t\toptions?: { cause?: unknown }'
		);
		const instance =
			"readonly kind: 'not_granted';\n\t\t\t\t\t\treadonly target: string;\n\t\t\t\t\t};\n\t\t\t\t\treadonly is_env_error";
		assert_mismatch(
			'env_error',
			'bootstrap',
			instance,
			instance.replace('readonly target', 'target')
		);
		assert_mismatch(
			'env_error',
			'bootstrap',
			"kind?: 'not_granted'\n\t\t\t\t\t) => error is {",
			'kind?: string\n\t\t\t\t\t) => error is {'
		);
		// the realm's `Error` not handed over
		const call = "op_gro_namespace<'@fuzdev/gro/env_error.ts'>(_scope, Error)";
		assert_mismatch('env_error', 'builtin', call, call.replace(', Error', ''));
		assert_mismatch(
			'env_error',
			'node',
			'readonly target: string;',
			'readonly target: string | undefined;'
		);
		assert_mismatch(
			'env_error',
			'node',
			'target: string, options?: ErrorOptions',
			'target: string, message?: string, options?: ErrorOptions'
		);
	});

	test('env', () => {
		// each method of the handle, claimed by the script, its parameters and results
		assert_mismatch(
			'env',
			'builtin',
			'\t\tget(name: string): string | undefined;\n',
			'\t\tget(name: string): string;\n'
		);
		assert_mismatch(
			'env',
			'builtin',
			'\t\tget_all(): Readonly<{ [name: string]: string }>;\n',
			'\t\tget_all(): { [name: string]: string };\n'
		);
		assert_mismatch(
			'env',
			'builtin',
			'\t\tnarrow(names: ReadonlyArray<string>): EnvHandle;\n',
			'\t\tnarrow(names: Array<string>): EnvHandle;\n'
		);
		// the root not of the handle's type, by the host's claim or the script's type argument
		assert_mismatch('env', 'bootstrap', 'readonly env: Handle;', 'readonly env: object;');
		// and the realm's `Error`, which `EnvError` extends, not handed over
		const call = "op_gro_namespace<'@fuzdev/gro/env.ts', EnvHandle>(_scope, Error)";
		assert_mismatch('env', 'builtin', call, call.replace(', EnvHandle', ''));
		assert_mismatch('env', 'builtin', call, call.replace(', Error)', ')'));
		assert_mismatch(
			'env',
			'node',
			'narrow(names: ReadonlyArray<string>): Env;',
			'narrow(names: ReadonlyArray<string>): object;'
		);
	});

	test('blake3', () => {
		// each member, claimed by the host: a function's parameters and results, and each method of
		// the hasher a factory returns
		const claim = (from: string, to: string) => assert_mismatch('blake3', 'bootstrap', from, to);
		claim(
			'readonly hash: (data: Uint8Array) => Uint8Array;',
			'readonly hash: (data: Uint8Array) => ArrayBuffer;'
		);
		claim(
			'readonly create_derive_key_hasher: (context: string) => {',
			'readonly create_derive_key_hasher: (context: unknown) => {'
		);
		claim(
			'readonly create_hasher: () => {\n\t\t\t\t\t\tupdate(data: Uint8Array): void;',
			'readonly create_hasher: () => {\n\t\t\t\t\t\tupdate(data: Uint8Array | string): void;'
		);
		claim(
			'readonly create_keyed_hasher: (key: Uint8Array) => {\n\t\t\t\t\t\tupdate(data: Uint8Array): void;\n\t\t\t\t\t\tfinalize(): Uint8Array;',
			'readonly create_keyed_hasher: (key: Uint8Array) => {\n\t\t\t\t\t\tupdate(data: Uint8Array): void;\n\t\t\t\t\t\tfinalize(): Uint8Array | undefined;'
		);
		// the namespace typed as another row's, and a second value handed over, where a row
		// of no error family's hands over its scope alone
		const call = "op_gro_namespace<'@fuzdev/gro/blake3.ts'>(_scope)";
		assert_mismatch('blake3', 'builtin', call, call.replace('blake3.ts', 'args.ts'));
		assert_mismatch('blake3', 'builtin', call, call.replace('(_scope)', '(_scope, _scope)'));
		assert_mismatch('blake3', 'node', 'finalize(): Uint8Array;', 'finalize(): ArrayBuffer;');
	});

	test('cwd', () => {
		// a string where the Node module has a function, and a function of a parameter
		assert_mismatch('cwd', 'bootstrap', 'readonly cwd: () => string;', 'readonly cwd: string;');
		assert_mismatch(
			'cwd',
			'bootstrap',
			'readonly cwd: () => string;',
			'readonly cwd: (_a: string) => string;'
		);
		assert_mismatch(
			'cwd',
			'node',
			'export declare const cwd: () => string;',
			'export declare const cwd: () => string | undefined;'
		);
	});

	test('exit', () => {
		// each function claimed by the host: a code required, and a result
		assert_mismatch(
			'exit',
			'bootstrap',
			'readonly exit: (code?: number) => never;',
			'readonly exit: (code: number) => never;'
		);
		assert_mismatch(
			'exit',
			'bootstrap',
			'readonly set_exit_code: (code: number) => void;',
			'readonly set_exit_code: (code: number) => number;'
		);
	});

	test('pid', () => {
		assert_mismatch('pid', 'bootstrap', 'readonly pid: number;', 'readonly pid: string;');
		assert_mismatch(
			'pid',
			'node',
			'export declare const pid: number;',
			'export declare const pid: bigint;'
		);
	});

	test('runtime_version', () => {
		assert_mismatch(
			'runtime_version',
			'bootstrap',
			'readonly runtime_version: string;',
			'readonly runtime_version: number;'
		);
		assert_mismatch(
			'runtime_version',
			'node',
			'export declare const runtime_version: string;',
			'export declare const runtime_version: `${string}/${string}`;'
		);
	});
});
