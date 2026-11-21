import { describe, test, assert } from 'vitest';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import ts from 'typescript';
import { paths } from './test_helpers.ts';

/*

A builtin module of the runtime has no source or types of its own: the host makes every member
natively from the native members' table, which `crates/gro_runtime/tests/native_members.txt`
pins line for line (a Rust test holds the two equal). This holds the table to each row's Node
module, `src/lib/<name>.ts`, read through the TypeScript compiler, both ways, so a member on one
side and not the other fails, naming it:

- the same exports, each the same kind of member: a function, data, a handle of the same kind,
  or a class
- each kind of handle the row makes, named in the table `<specifier>#<Interface>`, is the Node
  module's exported interface of that name, which the Node module's handles implement, or, when
  the module exports none of the name, the interface of the name an export's type is, declared
  in another module (`stderr`'s `ProcessOutput`, which `stdout.ts` declares), else TypeScript's
  own of it, from the compiler's lib (`AsyncIterableIterator`, which what `fs`'s `list` returns
  implements): the interface's members are the kind's methods
- each function and method of a `length` its declaration allows

It holds names, kinds, and arity, never argument or result types: the differential cases
(`crates/gro_cli/tests/host.rs` against what each Node module printed) hold what each member
does, the exact `length` of each function on both runtimes among it.

A `length` is JS's: the parameters before the first with a default or the rest parameter. A
signature with a body, Node's own function, gives its `length` exactly, an optional (`?`)
parameter without a default counted, as JS counts it. A signature without one, an interface's
method or an overload, can't say whether its implementation defaults an optional parameter, so
it allows each `length` from its required parameters to all of them before a rest; overloads
allow what any of them does.

On the Node side, the kinds of handle are the interfaces the module exports that have a method,
and those of TypeScript's lib the table names, and a value export is a class when it's declared one or is a constructor, a function when it has
a call signature, a handle when its type is one of those interfaces, and data otherwise. A
kind's members are the interface's own and those it inherits from an interface that is no kind
of the module's (`AsyncDisposable`), never one a kind it extends declares, which that kind's
prototype holds; an optional method (`AsyncIterator`'s `return?`) is a method, and a member keyed
by a well-known symbol is named as JS names its method (`[Symbol.asyncIterator]`), as the table
names it; a member that isn't a method is data of the kind, a handle when its type is one
of those interfaces (`fs.project`), which the table pins as `data of <kind>`: a value of a
record kind's one object, or a field each handle of the kind holds (`FsReadDir`'s `path`). A class
is held by its kind alone: its constructor and instances are the host's error class's, which the
differential cases hold.

The rows are read from the builtin table's source, never listed here, so a row can't join the
table without its Node module being compared.

*/

const TABLE_PATH = join(paths.root, 'crates/gro_runtime/src/resolver/builtin.rs');

const PINNED_PATH = join(paths.root, 'crates/gro_runtime/tests/native_members.txt');

const SPECIFIER_PREFIX = '@fuzdev/gro/';

interface BuiltinRow {
	/** The builtin's specifier, like `@fuzdev/gro/args.ts`. */
	specifier: string;
	/** The host module's name, like `args`. */
	name: string;
}

/**
 * The builtin table's rows, in its order, read from its source.
 */
const read_builtin_rows = (): Array<BuiltinRow> => {
	const source = readFileSync(TABLE_PATH, 'utf8');
	const rows = /macro_rules! builtin_rows \{([\s\S]*?)\n\}\n/.exec(source);
	assert.ok(rows, 'the table has its rows');
	return Array.from(rows[1]!.matchAll(/\$row!\(\s*"([^"]+)"/g), ([, specifier]) => {
		assert.ok(specifier!.startsWith(SPECIFIER_PREFIX) && specifier!.endsWith('.ts'), specifier);
		return {
			specifier: specifier!,
			name: specifier!.slice(SPECIFIER_PREFIX.length, -'.ts'.length)
		};
	});
};

/**
 * A row's members, each by where it goes and its name (`export args`,
 * `method of @fuzdev/gro/env.ts#Env get`, `data of @fuzdev/gro/fs.ts#FsReadDir path`), with
 * what it is: `function <length>` (the Node side's `function <least>..<most>` when its
 * declarations allow more than one), `data`, `handle <kind>`, or `class`.
 */
type Members = Map<string, string>;

/**
 * The pinned table's members of each row, by the row's specifier.
 * @param pinned - the pinned table's text, one member a line
 */
const read_pinned_members = (pinned: string): Map<string, Members> => {
	const rows: Map<string, Members> = new Map();
	for (const line of pinned.split('\n')) {
		if (line === '') continue;
		const [module, name, place, ...rest] = line.split(' ');
		assert.ok(module && name && place, line);
		// an export, or a member of a kind of handle, `<what> of <kind>` (`method of`)
		let where = place;
		if (place !== 'export') {
			assert.equal(rest.shift(), 'of', line);
			where = `${place} of ${rest.shift()}`;
		}
		const [made, detail] = rest;
		let what: string;
		if (made === 'length') {
			assert.match(detail!, /^\d+$/, line);
			what = `function ${detail}`;
		} else if (made === 'data') {
			what = 'data';
		} else if (made === 'handle') {
			assert.ok(detail, line);
			what = `handle ${detail}`;
		} else {
			assert.equal(made, 'class', line);
			what = 'class';
		}
		let members = rows.get(module);
		if (!members) rows.set(module, (members = new Map()));
		const key = `${where} ${name}`;
		assert.ok(!members.has(key), `${line}: pinned once`);
		members.set(key, what);
	}
	return rows;
};

/** The compiler options the Node modules are read with: their own, no Node types needed. */
const OPTIONS: ts.CompilerOptions = {
	target: ts.ScriptTarget.ESNext,
	module: ts.ModuleKind.NodeNext,
	moduleResolution: ts.ModuleResolutionKind.NodeNext,
	allowImportingTsExtensions: true,
	noEmit: true,
	strict: true,
	types: []
};

// the files every program reads from disk, parsed once
const source_files: Map<string, ts.SourceFile | undefined> = new Map();

/**
 * A program of the rows' Node modules, `replaced` standing in for a module's source by its
 * name, for a check that a mismatch is caught.
 */
const create_program = (
	rows: Array<BuiltinRow>,
	replaced: Record<string, string> = {}
): ts.Program => {
	const virtual = new Map(
		Object.entries(replaced).map(([name, text]) => [join(paths.lib, `${name}.ts`), text])
	);
	const host = ts.createCompilerHost(OPTIONS);
	const get_source_file = host.getSourceFile.bind(host);
	host.getSourceFile = (file_name, language_version, ...rest) => {
		const text = virtual.get(file_name);
		if (text !== undefined) return ts.createSourceFile(file_name, text, language_version, true);
		if (!source_files.has(file_name)) {
			source_files.set(file_name, get_source_file(file_name, language_version, ...rest));
		}
		return source_files.get(file_name);
	};
	return ts.createProgram(
		rows.map((row) => join(paths.lib, `${row.name}.ts`)),
		OPTIONS,
		host
	);
};

/**
 * The `length`s a function of `signature` may have, `[least, most]`: its parameters before the
 * first with a default or the rest parameter, `this` not among them, exactly when the
 * signature has a body, else from its required parameters to all of them.
 */
const to_lengths = (signature: ts.Signature): [number, number] => {
	const declaration = signature.getDeclaration();
	const exact = 'body' in declaration && declaration.body !== undefined;
	let least = 0;
	let most = 0;
	for (const parameter of declaration.parameters) {
		if (ts.isIdentifier(parameter.name) && parameter.name.text === 'this') continue;
		if (parameter.initializer || parameter.dotDotDotToken) break;
		most++;
		if (exact || !parameter.questionToken) least = most;
	}
	if (exact) least = most;
	return [least, most];
};

/**
 * What a function of `type` is: `function <length>`, or `function <least>..<most>` when its
 * declarations allow more than one, over all its call signatures.
 */
const to_function = (type: ts.Type): string => {
	const lengths = type.getCallSignatures().map(to_lengths);
	const least = Math.min(...lengths.map(([least]) => least));
	const most = Math.max(...lengths.map(([, most]) => most));
	return least === most ? `function ${least}` : `function ${least}..${most}`;
};

/**
 * The interfaces the table's kinds of `row` are named for, from its members' keys
 * (`method of <specifier>#<Interface> <name>`).
 */
const to_kind_names = (members: Members, row: BuiltinRow): Set<string> => {
	const names: Set<string> = new Set();
	const prefix = ` of ${row.specifier}#`;
	for (const key of members.keys()) {
		const at = key.indexOf(prefix);
		if (at !== -1) names.add(key.slice(at + prefix.length, key.indexOf(' ', at + prefix.length)));
	}
	return names;
};

/**
 * The members of `row`'s Node module, read from `program`, in the pinned table's form, a
 * function's as its lengths allow, the interfaces of TypeScript's lib among `kind_names`, the
 * table's, read as kinds of the row's.
 */
const read_node_members = (
	program: ts.Program,
	row: BuiltinRow,
	kind_names: Set<string>
): Members => {
	const checker = program.getTypeChecker();
	const source = program.getSourceFile(join(paths.lib, `${row.name}.ts`));
	assert.ok(source, row.name);
	const module = checker.getSymbolAtLocation(source);
	assert.ok(module, `${row.name} is a module`);
	const exports = checker
		.getExportsOfModule(module)
		.map((symbol) =>
			symbol.flags & ts.SymbolFlags.Alias ? checker.getAliasedSymbol(symbol) : symbol
		);
	// a member's type, an optional method's without its `undefined`
	const type_of_member = (property: ts.Symbol): ts.Type =>
		checker.getNonNullableType(checker.getTypeOfSymbol(property));
	const is_method = (property: ts.Symbol): boolean =>
		type_of_member(property).getCallSignatures().length > 0;
	// the kinds of handle: each exported interface with a method
	const kinds: Map<ts.Symbol, string> = new Map();
	for (const symbol of exports) {
		if (!(symbol.flags & ts.SymbolFlags.Interface)) continue;
		if (checker.getPropertiesOfType(checker.getDeclaredTypeOfSymbol(symbol)).some(is_method)) {
			kinds.set(symbol, `${row.specifier}#${symbol.name}`);
		}
	}
	// and each interface the table names a kind for that the module exports none of: the one an
	// export's type names, declared in another module (`stderr`'s `ProcessOutput`, `stdout.ts`'s),
	// else TypeScript's own
	const exported = new Set(Array.from(kinds.keys(), (symbol) => symbol.name));
	for (const symbol of exports) {
		if (!(symbol.flags & ts.SymbolFlags.Value)) continue;
		const type = checker.getTypeOfSymbol(symbol);
		const type_symbol = type.aliasSymbol ?? type.getSymbol();
		if (
			type_symbol &&
			type_symbol.flags & ts.SymbolFlags.Interface &&
			kind_names.has(type_symbol.name) &&
			!exported.has(type_symbol.name)
		) {
			kinds.set(type_symbol, `${row.specifier}#${type_symbol.name}`);
			exported.add(type_symbol.name);
		}
	}
	const globals = checker.getSymbolsInScope(source, ts.SymbolFlags.Interface);
	for (const name of kind_names) {
		if (exported.has(name)) continue;
		const symbol = globals.find(
			(global) =>
				global.name === name &&
				(global.declarations ?? []).length > 0 &&
				global.declarations!.every((declaration) =>
					program.isSourceFileDefaultLibrary(declaration.getSourceFile())
				)
		);
		if (symbol) kinds.set(symbol, `${row.specifier}#${name}`);
	}
	const members: Members = new Map();
	for (const symbol of exports) {
		if (!(symbol.flags & ts.SymbolFlags.Value)) continue;
		const type = checker.getTypeOfSymbol(symbol);
		const type_symbol = type.aliasSymbol ?? type.getSymbol();
		const kind = type_symbol && kinds.get(type_symbol);
		let what: string;
		if (symbol.flags & ts.SymbolFlags.Class || type.getConstructSignatures().length > 0) {
			what = 'class';
		} else if (type.getCallSignatures().length > 0) {
			what = to_function(type);
		} else if (kind) {
			what = `handle ${kind}`;
		} else {
			what = 'data';
		}
		members.set(`export ${symbol.name}`, what);
	}
	// the interface that declares `property`, by its symbol
	const declarer = (property: ts.Symbol): ts.Symbol | undefined => {
		const parent = property.declarations?.[0]?.parent;
		return parent && ts.isInterfaceDeclaration(parent)
			? checker.getSymbolAtLocation(parent.name)
			: undefined;
	};
	for (const [symbol, kind] of kinds) {
		for (const property of checker.getPropertiesOfType(checker.getDeclaredTypeOfSymbol(symbol))) {
			const from = declarer(property);
			// a member a kind it extends declares is that kind's
			if (from && from !== symbol && kinds.has(from)) continue;
			const name = checker.symbolToString(property);
			const property_type = type_of_member(property);
			if (is_method(property)) {
				members.set(`method of ${kind} ${name}`, to_function(property_type));
			} else {
				const type_symbol = property_type.aliasSymbol ?? property_type.getSymbol();
				const of_kind = type_symbol && kinds.get(type_symbol);
				members.set(`data of ${kind} ${name}`, of_kind ? `handle ${of_kind}` : 'data');
			}
		}
	}
	return members;
};

/**
 * Whether the Node module's `theirs` allows the table's `what`: the same, or a function of a
 * `length` in its range.
 */
const allows = (theirs: string, what: string): boolean => {
	if (theirs === what) return true;
	const range = /^function (\d+)\.\.(\d+)$/.exec(theirs);
	const length = /^function (\d+)$/.exec(what);
	return (
		!!range &&
		!!length &&
		Number(range[1]) <= Number(length[1]) &&
		Number(length[1]) <= Number(range[2])
	);
};

/**
 * How the table's members of a row differ from its Node module's, each naming the member:
 * one on one side only, or of another kind or `length`.
 */
const compare_members = (fgro: Members, node: Members): Array<string> => {
	const differences: Array<string> = [];
	for (const [key, what] of fgro) {
		const theirs = node.get(key);
		if (theirs === undefined) differences.push(`${key} (${what}) is on fgro only`);
		else if (!allows(theirs, what)) {
			differences.push(`${key} is ${what} on fgro, ${theirs} on Node`);
		}
	}
	for (const [key, what] of node) {
		if (!fgro.has(key)) differences.push(`${key} (${what}) is on Node only`);
	}
	return differences;
};

const rows = read_builtin_rows();

const read_pinned = (): string => readFileSync(PINNED_PATH, 'utf8');

/**
 * How `row`'s members differ, the pinned table's text as `pinned` and the Node modules'
 * sources with `replaced`.
 */
const differences_of = (
	row: BuiltinRow,
	pinned = read_pinned(),
	replaced: Record<string, string> = {}
): Array<string> => {
	const fgro = read_pinned_members(pinned).get(row.specifier) ?? new Map();
	const node = read_node_members(create_program(rows, replaced), row, to_kind_names(fgro, row));
	return compare_members(fgro, node);
};

describe('the builtin table', () => {
	test("the pinned members are the table's rows'", () => {
		const pinned = read_pinned_members(read_pinned());
		assert.deepEqual(
			Array.from(pinned.keys()).filter(
				(specifier) => !rows.some((row) => row.specifier === specifier)
			),
			[],
			'a pinned member of no row'
		);
		// each kind of member and of difference the checks below mutate is among them
		for (const name of ['cwd', 'exit', 'env', 'env_error', 'blake3', 'pid', 'fs', 'stderr']) {
			assert.ok(
				rows.some((row) => row.name === name),
				name
			);
		}
	});
});

describe.each(rows)('the $name builtin', (row) => {
	test("its members are the Node module's", () => {
		assert.deepEqual(differences_of(row), []);
	});
});

describe('a mismatch fails, naming the member', () => {
	const find_row = (name: string): BuiltinRow => {
		const row = rows.find((row) => row.name === name);
		assert.ok(row, name);
		return row;
	};

	/**
	 * Asserts the differences of `name` are exactly `expected` once `from` in the pinned table
	 * (`fgro`) or the Node module's source (`node`) is `to`.
	 */
	const assert_differences = (
		name: string,
		side: 'fgro' | 'node',
		from: string,
		to: string,
		expected: Array<string>
	): void => {
		const row = find_row(name);
		const pinned = read_pinned();
		const source = readFileSync(join(paths.lib, `${name}.ts`), 'utf8');
		const text = side === 'fgro' ? pinned : source;
		assert.include(text, from, 'the text to mutate is there');
		const mutated = text.replace(from, to);
		const differences =
			side === 'fgro'
				? differences_of(row, mutated)
				: differences_of(row, pinned, { [name]: mutated });
		assert.deepEqual(differences, expected, `${name}: ${from} as ${to}`);
	};

	test('a member on one side only', () => {
		assert_differences(
			'cwd',
			'fgro',
			'@fuzdev/gro/cwd.ts cwd export length 0 grant-free\n',
			'@fuzdev/gro/cwd.ts cwd export length 0 grant-free\n' +
				'@fuzdev/gro/cwd.ts extra export data grant-free\n',
			['export extra (data) is on fgro only']
		);
		assert_differences(
			'cwd',
			'node',
			'export const cwd = ',
			'export const extra = (): string => checked_cwd;\nexport const cwd = ',
			['export extra (function 0) is on Node only']
		);
		// a default export is a member too
		assert_differences('pid', 'node', 'export const pid', 'export default 1;\nexport const pid', [
			'export default (data) is on Node only'
		]);
		// a method on one side only
		assert_differences(
			'env',
			'fgro',
			'@fuzdev/gro/env.ts narrow method of @fuzdev/gro/env.ts#Env length 1 grant env throws EnvError\n',
			'',
			['method of @fuzdev/gro/env.ts#Env narrow (function 1) is on Node only']
		);
		assert_differences(
			'blake3',
			'node',
			'\tfinalize(): Uint8Array;\n',
			'\tfinalize(): Uint8Array;\n\treset(): void;\n',
			['method of @fuzdev/gro/blake3.ts#Blake3Hasher reset (function 0) is on Node only']
		);
	});

	test('a kind of handle on one side only', () => {
		// a kind the table names that the Node module has no interface of, and the interface
		assert_differences('blake3', 'fgro', '#Blake3Hasher length 1', '#Blake3Hashing length 1', [
			'method of @fuzdev/gro/blake3.ts#Blake3Hashing update (function 1) is on fgro only',
			'method of @fuzdev/gro/blake3.ts#Blake3Hasher update (function 1) is on Node only'
		]);
		// a kind of TypeScript's own is found by its name, and one of a name its lib lacks is none
		const listing = '@fuzdev/gro/fs.ts#AsyncIterableIterator';
		assert_differences(
			'fs',
			'fgro',
			`[Symbol.asyncIterator] method of ${listing} length 0`,
			`[Symbol.asyncIterator] method of ${listing}s length 0`,
			[
				`method of ${listing}s [Symbol.asyncIterator] (function 0) is on fgro only`,
				`method of ${listing} [Symbol.asyncIterator] (function 0) is on Node only`
			]
		);
		// its members are held as the module's are, an optional method's among them
		assert_differences(
			'fs',
			'fgro',
			`next method of ${listing} length 0`,
			`next method of ${listing} length 1`,
			[`method of ${listing} next is function 1 on fgro, function 0 on Node`]
		);
		assert_differences(
			'fs',
			'fgro',
			`@fuzdev/gro/fs.ts return method of ${listing} length 1 async grant fs\n`,
			'',
			[`method of ${listing} return (function 0..1) is on Node only`]
		);
		// a kind declared in another module is found through the export whose type it is, and is
		// none when the export isn't of it
		const output = '@fuzdev/gro/stderr.ts#ProcessOutput';
		assert_differences(
			'stderr',
			'node',
			'export const stderr: ProcessOutput',
			'export const stderr: Pick<ProcessOutput, "write">',
			[
				`export stderr is handle ${output} on fgro, data on Node`,
				`method of ${output} write (function 1) is on fgro only`,
				`data of ${output} is_terminal (data) is on fgro only`
			]
		);
	});

	test('a length changed', () => {
		assert_differences(
			'exit',
			'fgro',
			'set_exit_code export length 1',
			'set_exit_code export length 2',
			['export set_exit_code is function 2 on fgro, function 1 on Node']
		);
		// a default ends the length, as in JS; an optional parameter doesn't
		assert_differences(
			'exit',
			'node',
			'export const set_exit_code = (code: number): void',
			'export const set_exit_code = (code: number = 0): void',
			['export set_exit_code is function 1 on fgro, function 0 on Node']
		);
		assert_differences(
			'env_error',
			'node',
			'(error: unknown, kind?: EnvErrorKind)',
			'(error: unknown, ...kind: Array<EnvErrorKind>)',
			['export is_env_error is function 2 on fgro, function 1 on Node']
		);
		// a method's
		assert_differences(
			'blake3',
			'node',
			'\tupdate(data: Uint8Array): void;\n',
			'\tupdate(data: Uint8Array, more: number): void;\n',
			[
				'method of @fuzdev/gro/blake3.ts#Blake3Hasher update is function 1 on fgro, function 2 on Node'
			]
		);
	});

	test("an interface's optional parameter allows either length", () => {
		const optional = '\tupdate(data: Uint8Array, more?: number): void;\n';
		const update = '\tupdate(data: Uint8Array): void;\n';
		assert_differences('blake3', 'node', update, optional, []);
		// the same table, the method pinned of the longer length, passes too, and one past it fails
		const row = find_row('blake3');
		const source = readFileSync(join(paths.lib, 'blake3.ts'), 'utf8').replace(update, optional);
		const pinned = read_pinned();
		const method = '@fuzdev/gro/blake3.ts update method of @fuzdev/gro/blake3.ts#Blake3Hasher';
		assert.include(pinned, `${method} length 1`);
		assert.deepEqual(
			differences_of(row, pinned.replace(`${method} length 1`, `${method} length 2`), {
				blake3: source
			}),
			[]
		);
		assert.deepEqual(
			differences_of(row, pinned.replace(`${method} length 1`, `${method} length 3`), {
				blake3: source
			}),
			[
				'method of @fuzdev/gro/blake3.ts#Blake3Hasher update is function 3 on fgro, function 1..2 on Node'
			]
		);
	});

	test("a kind's members are its own", () => {
		// a member a kind it extends declares is that kind's, and data the table lacks is Node's only
		assert_differences(
			'env',
			'node',
			'export const env: Env',
			'export interface EnvChild extends Env {\n\tchild(): void;\n}\nexport const env: Env',
			['method of @fuzdev/gro/env.ts#EnvChild child (function 0) is on Node only']
		);
		assert_differences(
			'env',
			'node',
			'\tget_all(): Readonly<Record<string, string>>;\n',
			'\tget_all(): Readonly<Record<string, string>>;\n\treadonly size: number;\n',
			['data of @fuzdev/gro/env.ts#Env size (data) is on Node only']
		);
		// a field each handle holds, and a datum of a record kind that is a handle of a kind
		assert_differences(
			'fs',
			'fgro',
			'@fuzdev/gro/fs.ts path data of @fuzdev/gro/fs.ts#FsReadDir data grant fs\n',
			'',
			['data of @fuzdev/gro/fs.ts#FsReadDir path (data) is on Node only']
		);
		assert_differences(
			'fs',
			'node',
			'\treadonly project: FsDir;\n',
			'\treadonly project: FsReadDir;\n',
			[
				'data of @fuzdev/gro/fs.ts#Fs project is handle @fuzdev/gro/fs.ts#FsDir on fgro, handle @fuzdev/gro/fs.ts#FsReadDir on Node'
			]
		);
	});

	test('a kind changed', () => {
		assert_differences(
			'env',
			'fgro',
			'env export handle @fuzdev/gro/env.ts#Env',
			'env export data',
			['export env is data on fgro, handle @fuzdev/gro/env.ts#Env on Node']
		);
		assert_differences(
			'env_error',
			'fgro',
			'EnvError export class EnvError',
			'EnvError export data',
			['export EnvError is data on fgro, class on Node']
		);
		assert_differences(
			'pid',
			'node',
			'export const pid: number = globalThis.process.pid;',
			'export const pid = (): number => globalThis.process.pid;',
			['export pid is data on fgro, function 0 on Node']
		);
		assert_differences(
			'cwd',
			'node',
			'export const cwd = (): string => checked_cwd;',
			'export const cwd: string = checked_cwd;',
			['export cwd is function 0 on fgro, data on Node']
		);
	});
});
