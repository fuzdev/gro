import { describe, test, assert } from 'vitest';
import { readFileSync } from 'node:fs';
import { basename, join } from 'node:path';
import ts from 'typescript';

import { paths } from '#lib/paths.ts';

/*

Host modules' public types reference only ES and a few web types, never Node's,
so the API stays implementable on every runtime. This lint emits each host module's
declarations (`transpileDeclaration`, so also checking they need no inference), type-checks
them against TypeScript's ES and DOM libs with no `@types` loaded, and fails on:

- any import, re-export, `import()` type, or type reference directive
- `declare global`, which could reshape a lib type
- a type reference, heritage clause, `typeof` query, or `export default`/`export =` of a
  name whose symbol has a declaration outside the module and the ES libs, unless it's a
  DOM lib type on `WEB_TYPES`

With no Node types loaded, a Node name like `Buffer`, `process`, or `NodeJS` resolves
nowhere and is flagged; a DOM-only name like `HTMLElement` or `setTimeout` is flagged
because it isn't on the list.

*/

const HOST_MODULES = ['fs.ts'];

/**
 * The DOM lib types a host module's API may name. Each is in the WinterTC Minimum Common
 * API, so every server runtime has it, and the designed host modules use it: `AbortSignal`
 * for cancellation, the streams for spawned processes' stdio, and the fetch types plus
 * `WebSocket` and `EventSource` for `net`. Everything else in `lib.dom` is browser-only
 * or a global the runtime doesn't promise, so it isn't allowed until a module needs it.
 */
const WEB_TYPES: ReadonlySet<string> = new Set([
	'AbortSignal',
	'ReadableStream',
	'WritableStream',
	'URL',
	'Headers',
	'HeadersInit',
	'Request',
	'RequestInit',
	'Response',
	'BodyInit',
	'WebSocket',
	'EventSource'
]);

const is_dom_lib = (file: ts.SourceFile): boolean => basename(file.fileName).startsWith('lib.dom');

const to_declarations = (module_path: string): string => {
	const result = ts.transpileDeclaration(readFileSync(module_path, 'utf8'), {
		fileName: module_path,
		compilerOptions: { module: ts.ModuleKind.NodeNext, target: ts.ScriptTarget.ESNext }
	});
	const messages = (result.diagnostics ?? []).map((d) =>
		ts.flattenDiagnosticMessageText(d.messageText, '\n')
	);
	assert.deepEqual(messages, [], `declarations of ${module_path} need no inference`);
	return result.outputText;
};

/**
 * Finds the references in a module's emitted declarations to types outside the allowed libs.
 * @returns a description of each offending reference
 */
const find_foreign_type_references = (module_path: string, declarations: string): Array<string> => {
	const d_ts_path = module_path.replace(/\.ts$/, '.host_check.d.ts');
	const options: ts.CompilerOptions = {
		module: ts.ModuleKind.NodeNext,
		moduleResolution: ts.ModuleResolutionKind.NodeNext,
		target: ts.ScriptTarget.ESNext,
		lib: ['lib.esnext.d.ts', 'lib.dom.d.ts', 'lib.dom.iterable.d.ts'],
		types: [],
		strict: true,
		noEmit: true
	};
	const host = ts.createCompilerHost(options);
	const get_source_file = host.getSourceFile.bind(host);
	host.getSourceFile = (file_name, language_version, ...rest) =>
		file_name === d_ts_path
			? ts.createSourceFile(file_name, declarations, language_version, true)
			: get_source_file(file_name, language_version, ...rest);
	const file_exists = host.fileExists.bind(host);
	host.fileExists = (file_name) => file_name === d_ts_path || file_exists(file_name);
	const program = ts.createProgram([d_ts_path], options, host);
	const checker = program.getTypeChecker();
	const source = program.getSourceFile(d_ts_path);
	assert.ok(source);

	const is_allowed_declaration = (declaration: ts.Declaration, name: string): boolean => {
		const file = declaration.getSourceFile();
		if (file === source) return true;
		if (!program.isSourceFileDefaultLibrary(file)) return false;
		return !is_dom_lib(file) || WEB_TYPES.has(name);
	};

	const found: Array<string> = [];
	// flags `name` unless every declaration of its symbol is allowed
	const check_name = (name: ts.EntityName | ts.Expression): void => {
		let symbol = checker.getSymbolAtLocation(name);
		if (symbol && symbol.flags & ts.SymbolFlags.Alias) symbol = checker.getAliasedSymbol(symbol);
		const declarations = symbol?.declarations ?? [];
		const is_allowed =
			declarations.length > 0 && declarations.every((d) => is_allowed_declaration(d, symbol!.name));
		if (!is_allowed) {
			const from = declarations.map((d) => d.getSourceFile().fileName).join(', ') || 'nowhere';
			found.push(`\`${name.getText(source)}\` declared in ${from}`);
		}
	};

	for (const directive of source.typeReferenceDirectives) {
		found.push(`type reference directive "${directive.fileName}"`);
	}
	const visit = (node: ts.Node): void => {
		if (ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) {
			if (node.moduleSpecifier) found.push(`import of ${node.moduleSpecifier.getText(source)}`);
		} else if (ts.isImportTypeNode(node)) {
			found.push(`import type ${node.argument.getText(source)}`);
		} else if (ts.isModuleDeclaration(node) && node.flags & ts.NodeFlags.GlobalAugmentation) {
			found.push('`declare global`');
		} else if (ts.isTypeReferenceNode(node)) {
			check_name(node.typeName);
		} else if (ts.isExpressionWithTypeArguments(node)) {
			check_name(node.expression);
		} else if (ts.isExportAssignment(node) && ts.isIdentifier(node.expression)) {
			// `export default Buffer;` and `export = process;` export the global's type
			check_name(node.expression);
		} else if (ts.isTypeQueryNode(node)) {
			// `typeof a.b.c`: the leftmost name, unless it's `globalThis`, and the whole name
			let leftmost: ts.EntityName = node.exprName;
			while (ts.isQualifiedName(leftmost)) leftmost = leftmost.left;
			if (leftmost !== node.exprName && leftmost.text !== 'globalThis') check_name(leftmost);
			check_name(node.exprName);
		}
		ts.forEachChild(node, visit);
	};
	visit(source);
	return found;
};

const find_in_fixture = (declarations: string): Array<string> =>
	find_foreign_type_references(join(paths.lib, 'host_check_fixture.ts'), declarations);

describe('host module declarations', () => {
	for (const name of HOST_MODULES) {
		test(`${name} exports only ES and allowed web types`, () => {
			const module_path = join(paths.lib, name);
			const found = find_foreign_type_references(module_path, to_declarations(module_path));
			assert.deepEqual(found, []);
		});
	}

	test('allows ES types, listed web types, and the module own', () => {
		const found = find_in_fixture(
			[
				`export interface Ok { a: Uint8Array; b: ReadableStream; c: AbortSignal; d: Ok; e: AsyncDisposable }`,
				`export declare const ok: Ok;`,
				`export type K = keyof typeof ok;`,
				`export type U = typeof globalThis.Uint8Array;`,
				`export declare class C implements AsyncIterable<string> { [Symbol.asyncIterator](): AsyncIterator<string> }`
			].join('\n')
		);
		assert.deepEqual(found, []);
	});

	test('flags a Node type, a Node import, and an unresolved type', () => {
		const found = find_in_fixture(
			[
				`import type { Stats } from 'node:fs';`,
				`export declare const read: () => Promise<Buffer>;`,
				`export declare const stat: () => Stats;`,
				`export declare const missing: () => Missing;`
			].join('\n')
		);
		assert.deepEqual(found.length, 4, found.join('\n'));
		assert.ok(found[0]!.startsWith(`import of 'node:fs'`));
		assert.strictEqual(found[1], '`Buffer` declared in nowhere');
		assert.ok(found[2]!.startsWith('`Stats` declared in nowhere'));
		assert.strictEqual(found[3], '`Missing` declared in nowhere');
	});

	// each must be flagged at least once
	const MUTANTS: Record<string, string> = {
		typeof_process: 'export declare const p: typeof process;',
		typeof_buffer: 'export declare const b: InstanceType<typeof Buffer>;',
		keyof_env: 'export type K = keyof typeof process.env;',
		typeof_global: 'export declare const g: typeof globalThis.Buffer;',
		ret_type_query: 'export declare const q: ReturnType<typeof setTimeout>;',
		dom_only: 'export declare const el: () => HTMLElement;',
		dom_only_value: 'export declare const d: typeof document;',
		global_augmentation: 'declare global { interface Uint8Array { foo(): Uint8Array } } export {};',
		class_implements: 'export abstract class K implements NodeJS.EventEmitter {}',
		interface_extends: 'export interface I extends NodeJS.EventEmitter {}',
		class_member:
			'export declare class C { s?: import("node:fs").Stats; t(): Map<string, BufferEncoding> }',
		generic_argument: 'export interface A { x: Promise<Array<Buffer>> }',
		indexed_access: 'import type { Stats } from "node:fs"; export type S = Stats["size"];',
		method_return: 'export interface A { m(): NodeJS.ReadableStream }',
		reexport: 'export type { Stats } from "node:fs";',
		export_default_global: 'export default Buffer;',
		export_equals_global: 'export = process;',
		reference_directive: '/// <reference types="node" />\nexport declare const x: number;'
	};

	for (const [name, declarations] of Object.entries(MUTANTS)) {
		test(`flags the mutant ${name}`, () => {
			const found = find_in_fixture(declarations);
			assert.ok(found.length > 0, `expected a finding in: ${declarations}`);
		});
	}
});
