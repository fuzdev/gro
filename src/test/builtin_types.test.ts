import { describe, test, assert } from 'vitest';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import ts from 'typescript';

import { paths } from '#lib/paths.ts';

/*

A builtin module of the runtime (`crates/gro_runtime/js/<name>.ts`) declares no type of its own:
its exports have one declaration, the Node host module's (`src/lib/<name>.ts`). This checks, for
every row of the runtime's builtin table, the namespace the builtin's factory infers against that
module's exports, exactly and both ways, in a program shaped like the builtin's own: the builtin's
tsconfig, no Node types, and the Node module as its emitted declarations, which name no Node type
(`host_module_declarations.test.ts`).

The rows are read, never listed here, so a row can't join the table without its Node module
being compared: the factories from the bootstrap (`js/bootstrap.js`), which the runtime's own
tests hold to the table, and the table's specifiers from its source, which the two must match.

*/

const JS_DIR = join(paths.root, 'crates/gro_runtime/js');

const TABLE_PATH = join(paths.root, 'crates/gro_runtime/src/resolver/builtin.rs');

const SPECIFIER_PREFIX = '@fuzdev/gro/';

interface BuiltinRow {
	/** The builtin's specifier, like `@fuzdev/gro/args.ts`. */
	specifier: string;
	/** The host module's name, like `args`. */
	name: string;
	/** The factory's name, an export of `js/<name>.ts`. */
	factory: string;
}

/**
 * The bootstrap's factories, each row's specifier with the factory it names and the module
 * the bootstrap imports that factory from, which must be the row's own.
 */
const read_bootstrap_rows = (): Array<BuiltinRow> => {
	const source = readFileSync(join(JS_DIR, 'bootstrap.js'), 'utf8');
	const table = /gro_builtins = primordials\.ObjectFreeze\(\{([^}]*)\}\)/.exec(source);
	assert.ok(table, 'the bootstrap has its factories');
	const rows: Array<BuiltinRow> = [];
	for (const [, specifier, factory] of table[1]!.matchAll(/'([^']+)': (\w+)/g)) {
		assert.ok(specifier!.startsWith(SPECIFIER_PREFIX) && specifier!.endsWith('.ts'), specifier);
		const name = specifier!.slice(SPECIFIER_PREFIX.length, -'.ts'.length);
		assert.include(
			source,
			`import { ${factory} } from 'ext:gro/${name}.ts';`,
			`${factory} comes from the builtin's own module`
		);
		rows.push({ specifier: specifier!, name, factory: factory! });
	}
	return rows;
};

/**
 * The specifiers of the builtin table's rows, in its order, read from its source.
 */
const read_table_specifiers = (): Array<string> => {
	const source = readFileSync(TABLE_PATH, 'utf8');
	const rows = /macro_rules! builtin_rows \{([\s\S]*?)\n\}\n/.exec(source);
	assert.ok(rows, 'the table has its rows');
	return Array.from(rows[1]!.matchAll(/\$row!\(\s*"([^"]+)"/g), (match) => match[1]!);
};

/**
 * The check, a module beside the builtin: the namespace's exports are the Node module's and
 * `default`, each export exactly the Node module's, the two namespaces assign both ways, and
 * the factory takes one argument, its grant's scope, which no row's grant has.
 */
const to_check = ({ name, factory }: BuiltinRow): string => `
import type { ${factory} } from './${name}.ts';
import type * as node from './${name}.node_declarations.d.ts';

type Equal<A, B> = (<T>() => T extends A ? 1 : 2) extends <T>() => T extends B ? 1 : 2 ? true : false;
type Namespace = ReturnType<typeof ${factory}>;
type Builtin = Omit<Namespace, 'default'>;
type Node = typeof node;

export const exports: Equal<keyof Namespace, keyof Node | 'default'> = true;
export const no_default: Equal<Namespace['default'], undefined> = true;
export const members: Equal<{ [K in keyof Node]: Equal<Builtin[K], Node[K]> }[keyof Node], true> = true;
export const to_node = (builtin: Builtin): Node => builtin;
export const to_builtin = (node: Node): Builtin => node;
export const scope: Equal<Parameters<typeof ${factory}>, [_scope: undefined]> = true;
`;

// the files every check reads from disk, parsed once
const source_files: Map<string, ts.SourceFile | undefined> = new Map();

/**
 * Type-checks a row's check beside the builtin, against `declarations` standing in for the
 * Node module's, under the builtin's tsconfig.
 * @param row - the builtin's row
 * @param declarations - the Node module's declarations
 * @param builtin - the builtin's source in place of its file, when given
 * @returns the check's diagnostics, each as text
 */
const check_types = (row: BuiltinRow, declarations: string, builtin?: string): Array<string> => {
	const config_path = join(JS_DIR, 'tsconfig.json');
	const config = ts.readConfigFile(config_path, ts.sys.readFile);
	assert.ok(!config.error, 'the builtin tsconfig reads');
	const { options } = ts.parseJsonConfigFileContent(config.config, ts.sys, JS_DIR);
	const check_path = join(JS_DIR, `${row.name}.types_check.ts`);
	const virtual = new Map([
		[check_path, to_check(row)],
		[join(JS_DIR, `${row.name}.node_declarations.d.ts`), declarations]
	]);
	if (builtin !== undefined) virtual.set(join(JS_DIR, `${row.name}.ts`), builtin);
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
	const program = ts.createProgram([check_path, join(JS_DIR, 'internal.d.ts')], options, host);
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

const read_builtin = (name: string): string => readFileSync(join(JS_DIR, `${name}.ts`), 'utf8');

const rows = read_bootstrap_rows();

describe('the builtin table', () => {
	test("the rows compared are the table's", () => {
		assert.deepEqual(
			rows.map((row) => row.specifier),
			read_table_specifiers()
		);
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
		const source = read_builtin(row.name);
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
		// a factory that takes no scope, and one that takes another
		const scoped = '(_scope: undefined) =>';
		assert.include(source, scoped, 'the factory to mutate is there');
		assert.notDeepEqual(check_types(row, declarations, source.replace(scoped, '() =>')), []);
		assert.notDeepEqual(
			check_types(row, declarations, source.replace(scoped, '(_scope: string) =>')),
			[]
		);
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
		const source = read_builtin(name);
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
