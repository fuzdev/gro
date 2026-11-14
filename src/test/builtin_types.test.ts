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
(`crates/gro_runtime/js/builtin/<specifier>`) is a script, one function of the bootstrap object
and its grant's scope that returns the namespace. This checks, for every row of the runtime's
builtin table, the namespace that function infers against the Node module's exports, exactly and
both ways, and its parameters, in a program with the implementation's own declaration of the
bootstrap object, the ES library, no Node types, and the Node module as its emitted
declarations, which name no Node type (`host_module_declarations.test.ts`).

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
 * and the function takes the bootstrap object and one more argument, its grant's scope, which
 * no row's grant has.
 */
const to_check = ({ name }: BuiltinRow): string => `
import type { implementation } from './${name}.subject.ts';
import type * as node from './${name}.node_declarations.d.ts';

type Equal<A, B> = (<T>() => T extends A ? 1 : 2) extends <T>() => T extends B ? 1 : 2 ? true : false;
type Namespace = ReturnType<typeof implementation>;
type Builtin = Omit<Namespace, 'default'>;
type Node = typeof node;

export const exports: Equal<keyof Namespace, keyof Node | 'default'> = true;
export const no_default: Equal<Namespace['default'], undefined> = true;
export const members: Equal<{ [K in keyof Node]: Equal<Builtin[K], Node[K]> }[keyof Node], true> = true;
export const to_node = (builtin: Builtin): Node => builtin;
export const to_builtin = (node: Node): Builtin => node;
export const parameters: Equal<Parameters<typeof implementation>, [bootstrap: Bootstrap, _scope: undefined]> = true;
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
	implementation = read_implementation(row)
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
		[join(dir, `${row.name}.node_declarations.d.ts`), declarations]
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
		for (const name of ['args', 'cwd', 'pid', 'runtime_version']) {
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
		// every value the module exports is among them, so none escapes the widening
		assert.lengthOf(
			Array.from(declarations.matchAll(/^export declare /gm)),
			declared.length,
			'every export the check widens is a const declared on one line'
		);
		for (const [line, name, type] of declared) {
			const widened = declarations.replace(
				line,
				`export declare const ${name}: (${type}) | undefined;`
			);
			assert.notDeepEqual(check_types(row, widened), [], `${name} widened`);
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
		const made = 'default: undefined }';
		assert.include(source, made, 'the namespace to mutate is there');
		// an export the Node module doesn't have
		assert.notDeepEqual(
			check_types(row, declarations, source.replace(made, `default: undefined, extra: '' }`)),
			[]
		);
		// a `default` that isn't `undefined`
		assert.notDeepEqual(
			check_types(row, declarations, source.replace(made, 'default: null }')),
			[]
		);
		// an implementation that takes no scope, one that takes another, and one that takes
		// another bootstrap object
		const scoped = '(bootstrap: Bootstrap, _scope: undefined) {';
		assert.include(source, scoped, 'the parameters to mutate are there');
		for (const parameters of [
			'(bootstrap: Bootstrap) {',
			'(bootstrap: Bootstrap, _scope: string) {',
			'(bootstrap: Bootstrap & { extra: 1 }, _scope: undefined) {'
		]) {
			assert.notDeepEqual(
				check_types(row, declarations, source.replace(scoped, parameters)),
				[],
				parameters
			);
		}
	});
});

describe('a retyped export fails its check', () => {
	const find_row = (name: string): BuiltinRow => {
		const row = rows.find((row) => row.name === name);
		assert.ok(row, name);
		return row;
	};

	/**
	 * Asserts the check of `name` fails once `from` in the builtin's source or the Node
	 * module's declarations (`side`) is `to`.
	 */
	const assert_mismatch = (name: string, side: 'builtin' | 'node', from: string, to: string) => {
		const row = find_row(name);
		const declarations = node_declarations(name);
		const source = read_implementation(row);
		const text = side === 'builtin' ? source : declarations;
		assert.include(text, from, 'the text to mutate is there');
		const mutated = text.replace(from, to);
		const diagnostics =
			side === 'builtin' ? check_types(row, declarations, mutated) : check_types(row, mutated);
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
		// the array left mutable
		assert_mismatch('args', 'builtin', 'args: ObjectFreeze(op_gro_args())', 'args: op_gro_args()');
	});

	test('cwd', () => {
		// a string where the Node module has a function, and a function of a parameter
		assert_mismatch('cwd', 'builtin', 'create_record({ cwd,', "create_record({ cwd: '',");
		assert_mismatch(
			'cwd',
			'builtin',
			'const cwd = (): string =>',
			'const cwd = (_a: string): string =>'
		);
		assert_mismatch(
			'cwd',
			'node',
			'export declare const cwd: () => string;',
			'export declare const cwd: () => string | undefined;'
		);
	});

	test('pid', () => {
		assert_mismatch('pid', 'builtin', 'pid: op_gro_pid()', 'pid: String(op_gro_pid())');
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
			'builtin',
			'runtime_version: op_gro_runtime_version()',
			'runtime_version: 1'
		);
		assert_mismatch(
			'runtime_version',
			'node',
			'export declare const runtime_version: string;',
			'export declare const runtime_version: `${string}/${string}`;'
		);
	});
});
