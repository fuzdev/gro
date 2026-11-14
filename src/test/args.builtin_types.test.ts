import { describe, test, assert } from 'vitest';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import ts from 'typescript';

import { paths } from '#lib/paths.ts';

/*

The runtime's args builtin (`crates/gro_runtime/js/args.ts`) declares no type of its own: its
exports have one declaration, the Node host module's (`src/lib/args.ts`). This checks the
namespace the builtin's factory infers against that module's exports, exactly and both ways, in
a program shaped like the builtin's own: the builtin's tsconfig, no Node types, and the Node
module as its emitted declarations, which name no Node type (`host_module_declarations.test.ts`).

*/

const JS_DIR = join(paths.root, 'crates/gro_runtime/js');

const DECLARATIONS_PATH = join(JS_DIR, 'args.node_declarations.d.ts');

const BUILTIN_PATH = join(JS_DIR, 'args.ts');

/**
 * The check, a module beside the builtin: the namespace's exports are the Node module's and
 * `default`, each export exactly the Node module's, and the two namespaces assign both ways.
 */
const CHECK = `
import type { create_args_namespace } from './args.ts';
import type * as node from './args.node_declarations.d.ts';

type Equal<A, B> = (<T>() => T extends A ? 1 : 2) extends <T>() => T extends B ? 1 : 2 ? true : false;
type Namespace = ReturnType<typeof create_args_namespace>;
type Builtin = Omit<Namespace, 'default'>;
type Node = typeof node;

export const exports: Equal<keyof Namespace, keyof Node | 'default'> = true;
export const no_default: Equal<Namespace['default'], undefined> = true;
export const members: { [K in keyof Node]: Equal<Builtin[K], Node[K]> } = { args: true };
export const to_node = (builtin: Builtin): Node => builtin;
export const to_builtin = (node: Node): Builtin => node;
`;

/**
 * Type-checks `CHECK` beside the builtin, against `declarations` standing in for the Node
 * module's, under the builtin's tsconfig.
 * @param declarations - the Node module's declarations
 * @param builtin - the builtin's source in place of its file, when given
 * @returns the check's diagnostics, each as text
 */
const check_types = (declarations: string, builtin?: string): Array<string> => {
	const config_path = join(JS_DIR, 'tsconfig.json');
	const config = ts.readConfigFile(config_path, ts.sys.readFile);
	assert.ok(!config.error, 'the builtin tsconfig reads');
	const { options } = ts.parseJsonConfigFileContent(config.config, ts.sys, JS_DIR);
	const check_path = join(JS_DIR, 'args.types_check.ts');
	const virtual = new Map([
		[check_path, CHECK],
		[DECLARATIONS_PATH, declarations]
	]);
	if (builtin !== undefined) virtual.set(BUILTIN_PATH, builtin);
	const host = ts.createCompilerHost(options);
	const get_source_file = host.getSourceFile.bind(host);
	host.getSourceFile = (file_name, language_version, ...rest) => {
		const text = virtual.get(file_name);
		return text === undefined
			? get_source_file(file_name, language_version, ...rest)
			: ts.createSourceFile(file_name, text, language_version, true);
	};
	const file_exists = host.fileExists.bind(host);
	host.fileExists = (file_name) => virtual.has(file_name) || file_exists(file_name);
	const program = ts.createProgram([check_path, join(JS_DIR, 'internal.d.ts')], options, host);
	return ts
		.getPreEmitDiagnostics(program)
		.map((d) => ts.flattenDiagnosticMessageText(d.messageText, '\n'));
};

const node_declarations = (): string => {
	const module_path = join(paths.lib, 'args.ts');
	const result = ts.transpileDeclaration(readFileSync(module_path, 'utf8'), {
		fileName: module_path,
		compilerOptions: { module: ts.ModuleKind.NodeNext, target: ts.ScriptTarget.ESNext }
	});
	return result.outputText;
};

describe('the args builtin types', () => {
	test("the builtin's exports are exactly the Node module's", () => {
		assert.deepEqual(check_types(node_declarations()), []);
	});

	test('a mismatch fails the check', () => {
		const declarations = node_declarations();
		const declared = 'export declare const args: ReadonlyArray<string>;';
		assert.include(declarations, declared, 'the declaration to mutate is there');
		const mutable = declarations.replace(declared, 'export declare const args: Array<string>;');
		assert.notDeepEqual(check_types(mutable), []);
		const widened = declarations.replace(
			declared,
			'export declare const args: ReadonlyArray<string | number>;'
		);
		assert.notDeepEqual(check_types(widened), []);
		// an export the builtin doesn't have
		assert.notDeepEqual(check_types(`${declarations}\nexport declare const extra: string;\n`), []);
	});

	test("a mismatch on the builtin's side fails the check", () => {
		const declarations = node_declarations();
		const source = readFileSync(BUILTIN_PATH, 'utf8');
		const frozen = 'args: ObjectFreeze(op_gro_args())';
		assert.include(source, frozen, 'the export to mutate is there');
		// an export the Node module doesn't have
		const extra = source.replace(frozen, `${frozen}, extra: ''`);
		assert.notDeepEqual(check_types(declarations, extra), []);
		// the array left mutable
		const mutable = source.replace(frozen, 'args: op_gro_args()');
		assert.notDeepEqual(check_types(declarations, mutable), []);
	});
});
