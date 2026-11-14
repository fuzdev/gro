import { describe, test, assert } from 'vitest';
import { join } from 'node:path';
import ts from 'typescript';

import {
	BUILTIN_DIR,
	read_builtin_rows,
	read_implementation,
	type BuiltinRow
} from './builtin_test_helpers.ts';

/*

A builtin module's implementation (`crates/gro_runtime/js/builtin/<specifier>`) runs when a
run first imports the builtin, which can be after user code has patched the global intrinsics.
So it names no global and reads no property of an intrinsic: every intrinsic it uses comes
from `primordials`, a parameter. Two checks hold every row of the builtin table to that here,
and each is shown to fail on an implementation that breaks the rule:

- its own program (`builtin/tsconfig.json`), which has no library but `intrinsics.d.ts`: the
  types the compiler asks for, with no member, and no value. A global or a member of an
  intrinsic is a type error there. `cargo xtask check-js` runs the same program.
- a walk of its syntax for what that program accepts and still reaches an intrinsic or leaves
  the type check: iteration (`for`-`of`, a spread, an array pattern, each reading
  `Symbol.iterator`), `instanceof` (`Symbol.hasInstance`), `for`-`in` and `in` (which read the
  prototype chain), `globalThis` (the one global the compiler declares itself), `any`, a type
  assertion but `as const` (which can give a value members it wasn't declared with), and a
  `@ts-` directive.

Each is also the text the runtime's build takes: 7-bit ASCII, and line comments over one
function, or the binary's build or its start fails, which `gro check` doesn't run.

Neither sees a string made of an object, or a property read or written on an object whose
prototype is `Object.prototype`: the runtime's tests run every builtin in a realm whose
intrinsics are traps for those (`crates/gro_cli/tests/host.rs`).

*/

const CONFIG_PATH = join(BUILTIN_DIR, 'tsconfig.json');

const read_config = (): ts.ParsedCommandLine => {
	const config = ts.readConfigFile(CONFIG_PATH, ts.sys.readFile);
	assert.ok(!config.error, 'the builtin tsconfig reads');
	return ts.parseJsonConfigFileContent(config.config, ts.sys, BUILTIN_DIR);
};

// the files every check reads from disk, parsed once
const source_files: Map<string, ts.SourceFile | undefined> = new Map();

/**
 * Type-checks the implementations' program, the files its tsconfig includes.
 * @param row - the row whose implementation `implementation` stands in for, when given
 * @param implementation - the source in place of that row's file
 * @returns the program's diagnostics, each as text
 */
const check_program = (row?: BuiltinRow, implementation?: string): Array<string> => {
	const { options, fileNames } = read_config();
	const host = ts.createCompilerHost(options);
	const get_source_file = host.getSourceFile.bind(host);
	host.getSourceFile = (file_name, language_version, ...rest) => {
		if (row !== undefined && implementation !== undefined && file_name === row.path) {
			return ts.createSourceFile(file_name, implementation, language_version, true);
		}
		if (!source_files.has(file_name)) {
			source_files.set(file_name, get_source_file(file_name, language_version, ...rest));
		}
		return source_files.get(file_name);
	};
	return ts
		.getPreEmitDiagnostics(ts.createProgram(fileNames, options, host))
		.map((d) => ts.flattenDiagnosticMessageText(d.messageText, '\n'));
};

/**
 * What `implementation`'s syntax holds that reaches an intrinsic past the type check, or
 * leaves the check, each named.
 */
const find_unchecked_syntax = (implementation: string): Array<string> => {
	const source = ts.createSourceFile('implementation.ts', implementation, ts.ScriptTarget.ES2024);
	const found: Array<string> = [];
	if (/@ts-(?:ignore|expect-error|nocheck)/.test(implementation)) found.push('@ts- directive');
	const visit = (node: ts.Node): void => {
		if (ts.isForOfStatement(node)) found.push('for-of');
		if (ts.isSpreadElement(node)) found.push('spread');
		if (ts.isArrayBindingPattern(node)) found.push('array pattern');
		if (
			ts.isBinaryExpression(node) &&
			node.operatorToken.kind === ts.SyntaxKind.InstanceOfKeyword
		) {
			found.push('instanceof');
		}
		if (ts.isForInStatement(node)) found.push('for-in');
		if (ts.isBinaryExpression(node) && node.operatorToken.kind === ts.SyntaxKind.InKeyword) {
			found.push('in');
		}
		if (ts.isIdentifier(node) && node.text === 'globalThis') found.push('globalThis');
		if (node.kind === ts.SyntaxKind.AnyKeyword) found.push('any');
		if (
			(ts.isAsExpression(node) || ts.isTypeAssertionExpression(node)) &&
			!ts.isConstTypeReference(node.type)
		) {
			found.push('type assertion');
		}
		ts.forEachChild(node, visit);
	};
	visit(source);
	return found;
};

const rows = read_builtin_rows();

// what every implementation starts with, and the record it makes, where the mutants go
const TOP_LEVEL = '\tconst { core, primordials } = bootstrap;\n';
const IN_A_FUNCTION = 'ObjectAssign(ObjectCreate(null), fields);';

describe("the implementations' program", () => {
	test('has no library but its own declarations', () => {
		const { options, fileNames } = read_config();
		assert.strictEqual(options.noLib, true);
		assert.deepEqual(options.types, []);
		assert.includeMembers(fileNames, [
			join(BUILTIN_DIR, 'bootstrap.d.ts'),
			join(BUILTIN_DIR, 'intrinsics.d.ts'),
			...rows.map((row) => row.path)
		]);
		assert.lengthOf(fileNames, rows.length + 2);
	});

	test('type-checks', () => {
		assert.deepEqual(check_program(), []);
	});
});

describe.each(rows)('the $name implementation', (row) => {
	const source = read_implementation(row);

	/**
	 * Asserts the program fails once `from` in the implementation is `to`.
	 */
	const assert_refused = (from: string, to: string): void => {
		assert.include(source, from, 'the text to mutate is there');
		assert.notDeepEqual(check_program(row, source.replace(from, to)), [], to);
	};

	test('is 7-bit ASCII, and one function under line comments', () => {
		const non_ascii = Array.from(source).find((char) => char.charCodeAt(0) > 0x7f);
		assert.isUndefined(non_ascii, `non-ASCII ${JSON.stringify(non_ascii)}`);
		const lines = source.split('\n');
		const first = lines.findIndex((line) => line.trim() !== '' && !line.startsWith('//'));
		assert.isAbove(first, 0, 'comments come first');
		assert.strictEqual(lines[first], '(function (bootstrap: Bootstrap, _scope: undefined) {');
		assert.deepEqual(lines.slice(-2), ['});', '']);
		assert.notMatch(source, /^\s*(?:import|export)\b/m);
	});

	test('a global is refused, at the top level and in a function', () => {
		for (const global of [
			'Object.keys(primordials);',
			'globalThis.Object;',
			'JSON;',
			'Symbol.iterator;',
			'console;',
			'undefined_name;'
		]) {
			assert_refused(TOP_LEVEL, `${TOP_LEVEL}\t${global}\n`);
		}
		assert_refused(IN_A_FUNCTION, 'Object.assign(ObjectCreate(null), fields);');
		assert_refused(IN_A_FUNCTION, 'ObjectAssign(Object.create(null), fields);');
	});

	test('a member of an intrinsic is refused', () => {
		// a function's method and property, at the top level and in a function
		assert_refused(TOP_LEVEL, `${TOP_LEVEL}\tprimordials.ObjectCreate.name;\n`);
		assert_refused(IN_A_FUNCTION, 'ObjectAssign.call(undefined, ObjectCreate(null), fields);');
		assert_refused(IN_A_FUNCTION, 'ObjectAssign.bind(undefined)(ObjectCreate(null), fields);');
		// an array's, a string's, and an object's
		for (const member of [
			'[core].length;',
			'[core].map((item) => item);',
			'[core][0];',
			"'a'.slice(0);",
			'({ core }).toString();',
			'({ core }).constructor;'
		]) {
			assert_refused(TOP_LEVEL, `${TOP_LEVEL}\t${member}\n`);
		}
	});

	test('a promise and `this` are refused', () => {
		for (const use of ['(async () => 1)();', "import('x');", 'this;']) {
			assert_refused(TOP_LEVEL, `${TOP_LEVEL}\t${use}\n`);
		}
	});

	test('has no syntax the type check lets reach an intrinsic', () => {
		assert.deepEqual(find_unchecked_syntax(source), []);
		for (const [use, name] of [
			['for (const item of [core]) ObjectCreate(null);', 'for-of'],
			['ObjectCreate(...[null]);', 'spread'],
			['[...[core]];', 'spread'],
			['const [first] = [core];', 'array pattern'],
			['core instanceof primordials.Error;', 'instanceof'],
			['for (const key in core) ObjectCreate(null);', 'for-in'],
			["'ops' in core;", 'in'],
			['globalThis;', 'globalThis'],
			['(core as { ops: object }).ops;', 'type assertion'],
			['<object>core;', 'type assertion'],
			['((loose: any) => loose.ops)(core);', 'any'],
			['// @ts-expect-error\n\tcore.nope;', '@ts- directive']
		] as const) {
			const mutated = source.replace(TOP_LEVEL, `${TOP_LEVEL}\t${use}\n`);
			assert.notStrictEqual(mutated, source);
			assert.deepEqual(find_unchecked_syntax(mutated), [name], use);
		}
	});
});
