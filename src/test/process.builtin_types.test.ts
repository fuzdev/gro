import { describe, test, assert } from 'vitest';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import ts from 'typescript';

import { paths } from '#lib/paths.ts';

/*

The runtime's process builtin (`crates/gro_runtime/js/process.ts`) declares no interface of
its own: `Process` here, the Node implementation's, is the one declaration. This checks the
type the builtin's factory infers for what it carries against `Process`'s same members,
exactly (so `readonly` counts) and both ways, in a program shaped like the builtin's own:
the builtin's tsconfig, no Node types, and `src/lib/process.ts` as its emitted declarations,
which name no Node type (`host_module_declarations.test.ts`).

*/

/**
 * The `Process` members the builtin carries. A member joins by adding its name here.
 */
const CARRIED = ['args'];

const JS_DIR = join(paths.root, 'crates/gro_runtime/js');

/**
 * The check, a module beside the builtin: the namespace's exports, and each carried member of
 * `process` and the object as a whole, each exactly `Process`'s.
 */
const to_check = (carried: ReadonlyArray<string>): string => `
import type { create_process_namespace } from './process.ts';
import type { Process } from './process.node_declarations.d.ts';

type Equal<A, B> = (<T>() => T extends A ? 1 : 2) extends <T>() => T extends B ? 1 : 2 ? true : false;
type Namespace = ReturnType<typeof create_process_namespace>;
type Builtin = Namespace['process'];
type Carried = ${carried.map((name) => JSON.stringify(name)).join(' | ')};

export const exports: Equal<keyof Namespace, 'process' | 'default'> = true;
export const no_default: Equal<Namespace['default'], undefined> = true;
export const keys: Equal<keyof Builtin, Carried> = true;
export const members: { [K in Carried]: Equal<Builtin[K], Process[K]> } = {
	${carried.map((name) => `${JSON.stringify(name)}: true`).join(', ')}
};
export const whole: Equal<Builtin, Pick<Process, Carried>> = true;
export const to_node = (builtin: Builtin): Pick<Process, Carried> => builtin;
export const to_builtin = (node: Pick<Process, Carried>): Builtin => node;
`;

/**
 * Type-checks `check` beside the builtin, against `declarations` standing in for the Node
 * module's, under the builtin's tsconfig.
 * @returns the check's diagnostics, each as text
 */
const check_types = (check: string, declarations: string): Array<string> => {
	const config_path = join(JS_DIR, 'tsconfig.json');
	const config = ts.readConfigFile(config_path, ts.sys.readFile);
	assert.ok(!config.error, 'the builtin tsconfig reads');
	const { options } = ts.parseJsonConfigFileContent(config.config, ts.sys, JS_DIR);
	const check_path = join(JS_DIR, 'process.types_check.ts');
	const declarations_path = join(JS_DIR, 'process.node_declarations.d.ts');
	const virtual = new Map([
		[check_path, check],
		[declarations_path, declarations]
	]);
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
	const module_path = join(paths.lib, 'process.ts');
	const result = ts.transpileDeclaration(readFileSync(module_path, 'utf8'), {
		fileName: module_path,
		compilerOptions: { module: ts.ModuleKind.NodeNext, target: ts.ScriptTarget.ESNext }
	});
	return result.outputText;
};

describe('the process builtin types', () => {
	test("what the builtin carries is exactly Process's", () => {
		assert.deepEqual(check_types(to_check(CARRIED), node_declarations()), []);
	});

	test('a mismatch fails the check', () => {
		const declarations = node_declarations();
		const mutable = declarations.replace(
			'readonly args: ReadonlyArray<string>;',
			'args: ReadonlyArray<string>;'
		);
		assert.notEqual(mutable, declarations, 'the declaration to mutate is there');
		assert.notDeepEqual(check_types(to_check(CARRIED), mutable), []);
		const widened = declarations.replace(
			'readonly args: ReadonlyArray<string>;',
			'readonly args: ReadonlyArray<string | number>;'
		);
		assert.notDeepEqual(check_types(to_check(CARRIED), widened), []);
		// a member the builtin doesn't carry
		assert.notDeepEqual(check_types(to_check([...CARRIED, 'pid']), declarations), []);
	});
});
