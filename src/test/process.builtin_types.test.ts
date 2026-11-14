import { describe, test, assert } from 'vitest';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import ts from 'typescript';

import { paths } from '#lib/paths.ts';

/*

The runtime's process builtin (`crates/gro_runtime/js/process.ts`) declares no interface of
its own: each member it carries has one declaration, the Node host module's of the same name
(`args` in `src/lib/args.ts`). This checks the type the builtin's factory infers for each
carried member against that module's export, exactly and both ways, in a program shaped like
the builtin's own: the builtin's tsconfig, no Node types, and each Node module as its emitted
declarations, which name no Node type (`host_module_declarations.test.ts`).

*/

/**
 * The members the builtin carries, each the export of the Node host module named like it.
 * A member joins by adding its name here.
 */
const CARRIED = ['args'];

const JS_DIR = join(paths.root, 'crates/gro_runtime/js');

const to_declarations_path = (name: string): string =>
	join(JS_DIR, `${name}.node_declarations.d.ts`);

/**
 * The check, a module beside the builtin: the namespace's exports, and each carried member of
 * `process` and the object as a whole, each exactly the Node module's export.
 */
const to_check = (carried: ReadonlyArray<string>): string => `
import type { create_process_namespace } from './process.ts';
${carried.map((name) => `import type { ${name} } from './${name}.node_declarations.d.ts';`).join('\n')}

type Equal<A, B> = (<T>() => T extends A ? 1 : 2) extends <T>() => T extends B ? 1 : 2 ? true : false;
type Namespace = ReturnType<typeof create_process_namespace>;
type Builtin = Namespace['process'];
type Node = { readonly ${carried.map((name) => `${name}: typeof ${name}`).join('; readonly ')} };

export const exports: Equal<keyof Namespace, 'process' | 'default'> = true;
export const no_default: Equal<Namespace['default'], undefined> = true;
export const keys: Equal<keyof Builtin, keyof Node> = true;
export const members: { [K in keyof Node]: Equal<Builtin[K], Node[K]> } = {
	${carried.map((name) => `${JSON.stringify(name)}: true`).join(', ')}
};
export const whole: Equal<Builtin, Node> = true;
export const to_node = (builtin: Builtin): Node => builtin;
export const to_builtin = (node: Node): Builtin => node;
`;

/**
 * Type-checks `check` beside the builtin, against `declarations` standing in for the Node
 * modules', each by its module's name, under the builtin's tsconfig.
 * @returns the check's diagnostics, each as text
 */
const check_types = (check: string, declarations: ReadonlyMap<string, string>): Array<string> => {
	const config_path = join(JS_DIR, 'tsconfig.json');
	const config = ts.readConfigFile(config_path, ts.sys.readFile);
	assert.ok(!config.error, 'the builtin tsconfig reads');
	const { options } = ts.parseJsonConfigFileContent(config.config, ts.sys, JS_DIR);
	const check_path = join(JS_DIR, 'process.types_check.ts');
	const virtual = new Map([[check_path, check]]);
	for (const [name, text] of declarations) virtual.set(to_declarations_path(name), text);
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

const node_declarations = (name: string): string => {
	const module_path = join(paths.lib, `${name}.ts`);
	const result = ts.transpileDeclaration(readFileSync(module_path, 'utf8'), {
		fileName: module_path,
		compilerOptions: { module: ts.ModuleKind.NodeNext, target: ts.ScriptTarget.ESNext }
	});
	return result.outputText;
};

const to_declarations = (carried: ReadonlyArray<string>): Map<string, string> =>
	new Map(carried.map((name) => [name, node_declarations(name)]));

describe('the process builtin types', () => {
	test("what the builtin carries is exactly the Node modules'", () => {
		assert.deepEqual(check_types(to_check(CARRIED), to_declarations(CARRIED)), []);
	});

	test('a mismatch fails the check', () => {
		const declarations = node_declarations('args');
		const declared = 'export declare const args: ReadonlyArray<string>;';
		assert.include(declarations, declared, 'the declaration to mutate is there');
		const mutable = declarations.replace(declared, 'export declare const args: Array<string>;');
		assert.notDeepEqual(check_types(to_check(CARRIED), new Map([['args', mutable]])), []);
		const widened = declarations.replace(
			declared,
			'export declare const args: ReadonlyArray<string | number>;'
		);
		assert.notDeepEqual(check_types(to_check(CARRIED), new Map([['args', widened]])), []);
		// a member the builtin doesn't carry
		assert.notDeepEqual(
			check_types(to_check([...CARRIED, 'pid']), to_declarations([...CARRIED, 'pid'])),
			[]
		);
	});
});
