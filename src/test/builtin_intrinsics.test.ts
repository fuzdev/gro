import { describe, test, assert } from 'vitest';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

import {
	analyze_implementation,
	BOOTSTRAP_DECLARATIONS_PATH,
	find_unreviewed_declarations,
	INTRINSICS_DECLARATIONS_PATH,
	read_builtin_config,
	type ReviewedIntrinsics,
	type ReviewedMember
} from './builtin_constructs.ts';
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
from `primordials`, which the bootstrap object, its outer level's parameter, gives it. One
static check holds every row of the builtin table to that here (`builtin_constructs.ts`), and
each thing it refuses is shown to be refused, on an implementation that does it:

- its own program (`builtin/tsconfig.json`), which has no library but `intrinsics.d.ts`: the
  types the compiler asks for, with no member, and no value. A global or a member of an
  intrinsic is a type error there. `cargo xtask check-js` runs the same program.
- a walk of its syntax, with the program's types, for what that program accepts: a member of
  anything but what the bootstrap object gave, a property written, each construct that calls a
  protocol user code can answer (iteration, a string or number made of a value that isn't
  primitive, `await`, `instanceof`, `in`, `for`-`in`, a tagged template, `extends`, an
  accessor in an object literal), and what
  leaves the type check (`any`, a type assertion, a `@ts-` directive, `globalThis`, a `declare`,
  a reference to a file the reviewed declarations don't hold).

And what the bootstrap object gives is a reviewed list: `bootstrap.d.ts` declares the ops and
the primordials an implementation may use, and a test holds its declarations to the list
below, each primordial with why it's safe after user code ran, so one more is a deliberate
edit of both. `core` gives `ops` alone: no `loadExtScript`, and the bootstrap object no
`internals`.

Each is also the text the runtime's build takes: 7-bit ASCII, and line comments over one
function, or the binary's build or its start fails, which `gro check` doesn't run.

What no static check sees is which values reach a primordial, and what an intrinsic reads of
an object it's given: the runtime's tests run every builtin in a realm whose intrinsics are
traps, and call its functions with hostile arguments (`crates/gro_cli/tests/host.rs`).

*/

/**
 * The primordials an implementation may use, each reviewed: what it's declared as, and why
 * a call of it after user code patched the realm runs none of that code. A primordial is the
 * realm's own function, taken by name before user code ran, so what protects a call is what
 * the function does with its arguments, never that it's a copy.
 */
const REVIEWED_PRIMORDIALS: Record<string, ReviewedMember> = {
	Error: {
		type: 'ErrorConstructor',
		why: "constructed with a message, a string, and nothing else: the engine stores the string and captures the stack itself, and reads no option. What it makes is the realm's `Error`, as a caller expects of one thrown. Every row also hands it to the namespace op as it is, which makes it the host's classes' prototype natively and reads nothing of it."
	}
};

/**
 * What `intrinsics.d.ts` may declare beside interfaces with no member, each reviewed: a
 * member here is one every implementation may read after user code ran, and a declaration
 * that isn't an interface is a name every implementation may use.
 */
const REVIEWED_INTRINSICS: ReviewedIntrinsics = {
	members: {
		ErrorConstructor: [
			{
				text: 'new (message?: string): Error;',
				why: "the one way an implementation makes an error, with a message and no options, which is all `REVIEWED_PRIMORDIALS` reviews of `Error`: it's constructed, and no member of it or of what it makes is read."
			}
		]
	},
	others: [
		{
			text: 'type Readonly<T> = { readonly [K in keyof T]: T[K] };',
			why: 'a type the compiler asks for by name, and what `bootstrap.d.ts` declares a handle method to return: a type alone, which names no value.'
		}
	]
};

const rows = read_builtin_rows();

// what every implementation starts with, where the mutants go
const TOP_LEVEL = '\tconst { core, primordials } = bootstrap;\n';

// what a mutant hands a value to so the value is used: a function of its own, typed for any
// value, which reads nothing of it
const KEEP = '\tconst keep = <T>(value: T): T => value;\n\tkeep;\n';

/**
 * An implementation's start with `use` inside a function of its own, in place of `TOP_LEVEL`.
 */
const in_a_function = (use: string): string =>
	`${TOP_LEVEL}${KEEP}\tconst probe = (): unknown => ${use};\n\tprobe;\n`;

/**
 * The diagnostics of the implementations' program, with `implementation` in place of the
 * file of `row` when given.
 */
const check_program = (row: BuiltinRow, implementation?: string): Array<string> =>
	analyze_implementation(row, implementation).diagnostics;

describe("the implementations' program", () => {
	test('has no library but its own declarations', () => {
		const { options, fileNames } = read_builtin_config();
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
		assert.deepEqual(check_program(rows[0]!), []);
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
		assert.strictEqual(lines[first], '(function (bootstrap: Bootstrap) {');
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
			assert_refused(TOP_LEVEL, `${TOP_LEVEL}${KEEP}\t${global}\n`);
		}
		// a function that uses only what the implementation was given type-checks, so each
		// one below is refused for the global it names
		const allowed = in_a_function("keep(new primordials.Error('made'))");
		assert.deepEqual(check_program(row, source.replace(TOP_LEVEL, allowed)), []);
		assert_refused(TOP_LEVEL, in_a_function('Object.assign(core, core)'));
		assert_refused(TOP_LEVEL, in_a_function('keep(Object.create(null))'));
	});

	test('a member of an intrinsic is refused', () => {
		// a function's method and property, at the top level and in a function
		assert_refused(TOP_LEVEL, `${TOP_LEVEL}${KEEP}\tprimordials.Error.name;\n`);
		assert_refused(TOP_LEVEL, in_a_function("primordials.Error.call(undefined, 'made')"));
		assert_refused(TOP_LEVEL, in_a_function('primordials.Error.bind(undefined)'));
		// an array's, a string's, and an object's
		for (const member of [
			'[core].length;',
			'[core].map((item) => item);',
			'[core][0];',
			"'a'.slice(0);",
			'({ core }).toString();',
			'({ core }).constructor;'
		]) {
			assert_refused(TOP_LEVEL, `${TOP_LEVEL}${KEEP}\t${member}\n`);
		}
	});

	test('a promise and `this` are refused', () => {
		for (const use of ['(async () => 1)();', "import('x');", 'this;']) {
			assert_refused(TOP_LEVEL, `${TOP_LEVEL}${KEEP}\t${use}\n`);
		}
	});

	test('holds nothing the walk refuses, and each is refused', () => {
		assert.deepEqual(analyze_implementation(row, source).refused, []);
		// each use, what the walk names it, and whether the program accepts it, so the walk
		// alone refuses it
		for (const [use, refused, accepted] of [
			// iteration
			['for (const item of [core]) keep(item);', ['for-of'], true],
			['keep(...[null]);', ['spread'], true],
			['[...[core]];', ['spread'], true],
			['const [first] = [core];\n\tkeep(first);', ['array pattern'], true],
			['let first;\n\t[first] = [core];\n\tkeep(first);', ['destructuring assignment'], true],
			['(function* () {\n\t\tyield 1;\n\t});', ['generator'], false],
			['({ ...core });', ['object spread'], true],
			// an accessor in a literal, which reading the literal calls
			['keep({\n\t\tget ops() {\n\t\t\treturn 1;\n\t\t}\n\t});', ['accessor'], true],
			['keep({\n\t\tset ops(_value: number) {}\n\t});', ['accessor'], true],
			// the other protocols
			['core instanceof primordials.Error;', ['instanceof'], true],
			['for (const key in core) keep(key);', ['for-in'], true],
			["'ops' in core;", ['in'], true],
			['(async () => {\n\t\tawait 1;\n\t});', ['async', 'await'], false],
			["import('x');", ['any', 'import'], false],
			["import.defer('x');", ['import'], false],
			['using held = core;\n\tkeep(held);', ['using'], false],
			[
				'(async () => {\n\t\tawait using held = core;\n\t\theld.ops;\n\t});',
				['async', 'member', 'using'],
				false
			],
			['keep``;', ['tagged template'], false],
			['class Failure extends primordials.Error {}\n\tnew Failure();', ['extends'], true],
			// a string or a number made of a value that isn't primitive
			['`${core}`;', ['coercion'], true],
			["'' + core;", ['coercion'], true],
			["let text = '';\n\ttext += core;\n\tkeep(text);", ['coercion'], true],
			['core == bootstrap.core;', ['coercion'], true],
			['core < bootstrap.core;', ['coercion'], true],
			['+core;', ['coercion'], true],
			['({ [keep({ __proto__: null })]: 1 });', ['coercion'], false],
			// of a value whose type says nothing, or not enough, of what it is
			['((given: unknown) => `${given}`);', ['coercion'], true],
			['((given: string | object) => `${given}`);', ['coercion'], true],
			['(<T>(given: T) => `${given}`);', ['coercion'], true],
			['((given: string & { brand: true }) => `${given}`);', ['coercion'], true],
			["((loose: any) => '' + loose);", ['any', 'coercion'], true],
			// of what a function typed to return nothing returned
			['const quiet: () => void = () => core;\n\tkeep(`${quiet()}`);', ['coercion'], true],
			// of a binding a closure assigns after its type was checked
			[
				"let text: string | object = '';\n\tconst set = (): void => {\n\t\ttext = core;\n\t};\n\tset();\n\tkeep(`${text}`);",
				['coercion'],
				true
			],
			[
				"let text: string | object = '';\n\tconst set = (): void => {\n\t\ttext ||= core;\n\t};\n\tset();\n\tkeep(`${text}`);",
				['coercion'],
				true
			],
			// a member of a value the bootstrap object didn't give
			['({ core }).core;', ['member'], true],
			["((given: { name: string }) => given.name)({ name: '' });", ['member'], true],
			["(({ name }: { name: string }) => name)({ name: '' });", ['member'], true],
			["const { name } = { name: '' };\n\tkeep(name);", ['member'], true],
			["const made = { name: '' };\n\tmade.name;", ['member'], true],
			['let ops = core;\n\tops = core;\n\tops.ops;', ['member'], true],
			['core?.ops;', ['member'], true],
			['for (const item of [core]) item.ops;', ['for-of', 'member'], true],
			['const [first] = core;\n\tfirst.ops;', ['array pattern', 'member'], false],
			["const key = 'ops';\n\tconst { [key]: ops } = core;\n\tkeep(ops);", ['member'], true],
			["core['ops'];", ['member'], true],
			['const { ops = 1 } = core;\n\tkeep(ops);', ['member'], true],
			// a rest of what the bootstrap object gave, which holds what no declaration names
			[
				'const { core: held, ...others } = bootstrap;\n\tkeep(held);\n\tkeep(others);',
				['object rest'],
				true
			],
			['const {\n\t\tcore: { ...all }\n\t} = bootstrap;\n\tkeep(all);', ['object rest'], true],
			["const { ...copy } = { name: '' };\n\tkeep(copy);", ['member', 'object rest'], true],
			[
				'class Handle {\n\t\tbrand = 1;\n\t\tread(): number {\n\t\t\treturn this.brand;\n\t\t}\n\t}\n\tnew Handle();',
				['member'],
				true
			],
			// a property written
			["const made = { name: '' };\n\tmade.name = '';", ['member', 'property write'], true],
			['const made = { count: 0 };\n\tmade.count++;', ['member', 'property write'], true],
			['core.ops = core.ops;', ['property write'], false],
			['delete core.ops;', ['property write'], false],
			["core['ops'] = core.ops;", ['member', 'property write'], false],
			['const made = { count: 0 };\n\tmade.count--;', ['member', 'property write'], true],
			['const made = { count: 0 };\n\t++made.count;', ['member', 'property write'], true],
			['const made = { count: 0 };\n\tmade.count += 1;', ['member', 'property write'], true],
			[
				'let held;\n\t({ core: held } = bootstrap);\n\tkeep(held);',
				['destructuring assignment'],
				true
			],
			['function* make() {\n\t\tyield 1;\n\t}\n\tmake;', ['generator'], false],
			['({\n\t\t*make() {\n\t\t\tyield 1;\n\t\t}\n\t});', ['generator'], false],
			// what leaves the type check
			['globalThis;', ['globalThis'], true],
			['(function () {\n\t\treturn arguments;\n\t});', ['arguments'], true],
			['(core as { ops: object }).ops;', ['member', 'type assertion'], true],
			[
				'(keep({ __proto__: null }) as unknown as { slice(): string }).slice();',
				['member', 'type assertion'],
				true
			],
			['<object>core;', ['type assertion'], true],
			['core!;', ['type assertion'], true],
			['((loose: any) => loose.ops)(core);', ['any', 'member'], true],
			['// @ts-expect-error\n\tcore.nope;', ['@ts- directive'], true],
			['// @ts-ignore\n\tcore.nope;', ['@ts- directive'], true],
			['glob\\u0061lThis;', ['globalThis'], true],
			// a call no signature types gives `any`, which is assigned to anything
			[
				"((given: unknown) => {\n\t\tif (typeof given !== 'function') return '';\n\t\tconst text: string = given();\n\t\treturn 'a' + text;\n\t});",
				['any'],
				true
			],
			[
				'declare const structuredClone: (value: object) => object;\n\tstructuredClone(core);',
				['declare'],
				false
			],
			[
				'function to_text(value: unknown): string;\n\tfunction to_text(value: unknown): unknown {\n\t\treturn value;\n\t}\n\t`${to_text(core)}`;',
				['overload'],
				true
			],
			[
				'class Reader {\n\t\tread(value: unknown): string;\n\t\tread(value: unknown): unknown {\n\t\t\treturn value;\n\t\t}\n\t}\n\tnew Reader();',
				['overload'],
				true
			],
			[
				'const is_text = (value: unknown): value is string => value === core;\n\tconst made: unknown = core;\n\tif (is_text(made)) `${made}`;',
				['type predicate'],
				true
			],
			['bootstrap = { core, primordials };', ['parameter write'], true],
			// what it doesn't refuse: a member of what the bootstrap object gave, by a pattern
			// or a name, a private name, a spread of a literal, and a string made of primitives
			['bootstrap.core.ops;', [], true],
			['const { ops } = core;\n\tkeep(ops);', [], true],
			['const given = core;\n\tgiven.ops;', [], true],
			[
				'class Handle {\n\t\t#brand = 1;\n\t\tread(): number {\n\t\t\treturn this.#brand;\n\t\t}\n\t}\n\tnew Handle();',
				[],
				true
			],
			[
				'class Handle {\n\t\t#brand = 1;\n\t\tstatic is(value: object): boolean {\n\t\t\treturn #brand in value;\n\t\t}\n\t}\n\tkeep(Handle);',
				[],
				true
			],
			['({ ...{ core } });', [], true],
			["keep('made' as const);", [], true],
			["`${1}${''}`;", [], true],
			['`${true}${1n}${null}${undefined}`;', [], true],
			["((given: unknown) => (typeof given === 'string' ? `${given}` : ''));", [], true],
			['((key: symbol) => ({ __proto__: null, [key]: 1 }));', [], true],
			['((count: number) => {\n\t\tcount = 1;\n\t\treturn count;\n\t});', [], true],
			['((callback: () => number) => callback() + 1);', [], true],
			[
				'interface Shape {\n\t\tread(): number;\n\t}\n\tclass Handle implements Shape {\n\t\tread(): number {\n\t\t\treturn 1;\n\t\t}\n\t}\n\tnew Handle();',
				[],
				true
			],
			["1 + 1 < 3 === ('' + 1 === '1');", [], true]
		] satisfies Array<[string, Array<string>, boolean]>) {
			const mutated = source.replace(TOP_LEVEL, `${TOP_LEVEL}${KEEP}\t${use}\n`);
			assert.notStrictEqual(mutated, source);
			const analysis = analyze_implementation(row, mutated);
			assert.deepEqual(analysis.refused, refused, use);
			assert.strictEqual(analysis.diagnostics.length === 0, accepted, `${use}: the program`);
		}
	});

	test('each operator that makes a string or a number of a value that is not one is refused', () => {
		const refused = (use: string): Array<string> =>
			analyze_implementation(row, source.replace(TOP_LEVEL, `${TOP_LEVEL}${KEEP}\t${use}\n`))
				.refused;
		const binary = ['+', '-', '*', '**', '/', '%', '<<', '>>', '>>>', '&', '|', '^'];
		for (const operator of [...binary, '==', '!=', '<', '<=', '>', '>=']) {
			// either operand
			assert.deepEqual(refused(`core ${operator} 1;`), ['coercion'], `core ${operator} 1`);
			assert.deepEqual(refused(`1 ${operator} core;`), ['coercion'], `1 ${operator} core`);
		}
		for (const operator of binary) {
			const use = `let made = 1;\n\tmade ${operator}= core;\n\tkeep(made);`;
			assert.deepEqual(refused(use), ['coercion'], `${operator}=`);
		}
		for (const operator of ['+', '-', '~', '++', '--']) {
			assert.deepEqual(refused(`let held = core;\n\t${operator}held;`), ['coercion'], operator);
		}
		for (const operator of ['++', '--']) {
			assert.deepEqual(refused(`let held = core;\n\theld${operator};`), ['coercion'], operator);
		}
		// and none of them of numbers
		for (const operator of binary) {
			const use = `let made = 1;\n\tmade ${operator}= 2 ${operator} 3;\n\tkeep(-made);`;
			assert.deepEqual(refused(use), [], use);
		}
	});

	test("what turns the program's check off, or adds to its declarations, is refused", () => {
		const called = source.replace(TOP_LEVEL, `${TOP_LEVEL}\tstructuredClone(core);\n`);
		// a global the program lacks, called with no member read: the walk refuses the call
		assert.deepEqual(analyze_implementation(row, called).refused, ['any']);
		assert.notDeepEqual(analyze_implementation(row, called).diagnostics, []);
		// the compiler reads the directive that turns its check of a file off in any case
		for (const directive of ['// @ts-nocheck', '// @TS-NOCHECK', '// @Ts-NoCheck']) {
			const analysis = analyze_implementation(row, `${directive}\n${called}`);
			assert.deepEqual(analysis.diagnostics, [], directive);
			assert.deepEqual(analysis.refused, ['@ts- directive', 'any'], directive);
		}
		// a file of declarations the reviewed two don't hold, named from the implementation
		// or from one of them, and a library
		const elsewhere = `/// <reference path="${join(BUILTIN_DIR, '..', 'internal.d.ts')}" />\n`;
		assert.deepEqual(analyze_implementation(row, elsewhere + source).refused, [
			'other file',
			'reference directive'
		]);
		const declarations = readFileSync(BOOTSTRAP_DECLARATIONS_PATH, 'utf8');
		assert.deepEqual(analyze_implementation(row, source, elsewhere + declarations).refused, [
			'other file'
		]);
		assert.deepEqual(
			analyze_implementation(row, `/// <reference lib="es2024" />\n${source}`).refused,
			['reference directive']
		);
	});

	test('with a promise declared, the walk alone refuses what makes one', () => {
		// a row whose exports return promises declares the type, and the program then accepts
		// an `async` function that awaits nothing and types an `import()`
		const declarations = `${readFileSync(BOOTSTRAP_DECLARATIONS_PATH, 'utf8')}interface Promise<T> {}\n`;
		for (const [use, refused, accepted] of [
			['(async () => 1);', ['async'], true],
			['(async () => {\n\t\tawait 1;\n\t});', ['async', 'await'], false],
			["import('x');", ['import'], false]
		] satisfies Array<[string, Array<string>, boolean]>) {
			const mutated = source.replace(TOP_LEVEL, `${TOP_LEVEL}${KEEP}\t${use}\n`);
			const analysis = analyze_implementation(row, mutated, declarations);
			assert.deepEqual(analysis.refused, refused, use);
			assert.strictEqual(analysis.diagnostics.length === 0, accepted, `${use}: the program`);
		}
	});

	test('a member declared optional is refused, of the bootstrap object too', () => {
		const declarations = readFileSync(BOOTSTRAP_DECLARATIONS_PATH, 'utf8');
		const declared = '\treadonly core: {\n';
		assert.include(declarations, declared, 'the declaration to mutate is there');
		// read as a member, and bound by a pattern
		for (const use of ['bootstrap.maybe;', 'const { maybe } = bootstrap;\n\tkeep(maybe);']) {
			const mutated = source.replace(TOP_LEVEL, `${TOP_LEVEL}${KEEP}\t${use}\n`);
			for (const [member, refused] of [
				['readonly maybe: object;', []],
				['readonly maybe?: object;', ['member']]
			] satisfies Array<[string, Array<string>]>) {
				const analysis = analyze_implementation(
					row,
					mutated,
					declarations.replace(declared, `\t${member}\n${declared}`)
				);
				assert.deepEqual(analysis.diagnostics, [], member);
				assert.deepEqual(analysis.refused, refused, `${use} with ${member}`);
			}
		}
	});

	test('a script that is not one function is refused', () => {
		for (const other of [`${source}(function () {});\n`, 'const made = 1;\n', '(() => 1);\n']) {
			assert.include(analyze_implementation(row, other).refused, 'not one function', other);
		}
	});
});

describe('what the bootstrap object gives', () => {
	const bootstrap = readFileSync(BOOTSTRAP_DECLARATIONS_PATH, 'utf8');
	const intrinsics = readFileSync(INTRINSICS_DECLARATIONS_PATH, 'utf8');

	const find = (
		declared = bootstrap,
		declared_intrinsics = intrinsics,
		reviewed_intrinsics = REVIEWED_INTRINSICS
	): Array<string> =>
		find_unreviewed_declarations(
			declared,
			declared_intrinsics,
			REVIEWED_PRIMORDIALS,
			reviewed_intrinsics
		);

	/**
	 * `text` with `from` replaced by `to`, `from` being there.
	 */
	const mutate = (text: string, from: string, to: string): string => {
		assert.include(text, from, 'the declaration to mutate is there');
		return text.replace(from, to);
	};

	test('is the reviewed ops and primordials, and no more', () => {
		assert.deepEqual(find(), []);
		for (const { why } of Object.values(REVIEWED_PRIMORDIALS)) assert.isAbove(why.length, 40);
	});

	test("the intrinsics' declarations are the reviewed ones, and no more", () => {
		const { members, others } = REVIEWED_INTRINSICS;
		for (const { why } of [...Object.values(members).flat(), ...others]) {
			assert.isAbove(why.length, 40);
		}
		// what is declared and not reviewed: a member, and a declaration that isn't an interface
		assert.deepEqual(find(bootstrap, intrinsics, { members: {}, others }), [
			'intrinsics.d.ts ErrorConstructor'
		]);
		assert.deepEqual(find(bootstrap, intrinsics, { members, others: [] }), [
			'intrinsics.d.ts: type Readonly<T> = { readonly [K in keyof T]: T[K] };'
		]);
		// what is reviewed and not declared: an interface's members, one member of two, and a
		// declaration
		const reviewed = { text: 'new (message?: string): Error;', why: '' };
		assert.deepEqual(
			find(bootstrap, intrinsics, {
				members: { ...members, TypeErrorConstructor: [reviewed] },
				others
			}),
			['intrinsics.d.ts TypeErrorConstructor is reviewed and gone']
		);
		assert.deepEqual(
			find(bootstrap, intrinsics, {
				members: { ErrorConstructor: [reviewed, { text: 'readonly prototype: Error;', why: '' }] },
				others
			}),
			['intrinsics.d.ts ErrorConstructor']
		);
		assert.deepEqual(
			find(bootstrap, intrinsics, {
				members,
				others: [...others, { text: 'type Partial<T> = { [K in keyof T]?: T[K] };', why: '' }]
			}),
			['intrinsics.d.ts: type Partial<T> = { [K in keyof T]?: T[K] }; is reviewed and gone']
		);
	});

	test('an unreviewed declaration is found', () => {
		const core = '\treadonly core: {\n';
		const ops = '\t\treadonly ops: {\n';
		const primordials = '\treadonly primordials: {\n';
		for (const [mutated, found] of [
			// the object every extension script shares, and the bootstrap object's other members
			[mutate(bootstrap, core, `\treadonly internals: object;\n${core}`), ['Bootstrap.internals']],
			// a script loaded from an implementation, and any other function of `core`
			[
				mutate(
					bootstrap,
					ops,
					`\t\treadonly loadExtScript: (specifier: string) => object;\n${ops}`
				),
				['core.loadExtScript']
			],
			// an op that isn't a host op of gro's, and one that isn't a function
			[
				mutate(bootstrap, ops, `${ops}\t\t\treadonly op_eval_context: () => object;\n`),
				['core.ops.op_eval_context']
			],
			[
				mutate(bootstrap, ops, `${ops}\t\t\treadonly op_gro_made: { call: () => object };\n`),
				['core.ops.op_gro_made']
			],
			// a primordial nothing reviewed, and a reviewed one declared otherwise
			[
				mutate(
					bootstrap,
					primordials,
					`${primordials}\t\treadonly ArrayPrototypeMap: (list: object) => object;\n`
				),
				['primordials.ArrayPrototypeMap']
			],
			[
				mutate(bootstrap, 'readonly Error: ErrorConstructor;', 'readonly Error: object;'),
				['primordials.Error: object']
			],
			// an optional member, and a method
			[
				mutate(bootstrap, primordials, `${primordials}\t\treadonly Maybe?: () => object;\n`),
				['Bootstrap.primordials: readonly Maybe?: () => object;']
			],
			[mutate(bootstrap, core, `\tload(): object;\n${core}`), ['Bootstrap: load(): object;']],
			// a reviewed primordial that's no longer declared
			[
				mutate(bootstrap, '\t\treadonly Error: ErrorConstructor;\n', ''),
				['primordials.Error is reviewed and gone']
			],
			// a second declaration, another name, and members taken from another interface
			[
				`${bootstrap}interface Extra {}\n`,
				['bootstrap.d.ts declares more or less than the interface `Bootstrap`']
			],
			[
				mutate(bootstrap, 'interface Bootstrap {', 'interface Other {'),
				['bootstrap.d.ts declares more or less than the interface `Bootstrap`']
			],
			[
				mutate(
					bootstrap,
					'interface Bootstrap {',
					'interface Bootstrap extends ErrorConstructor {'
				),
				['bootstrap.d.ts declares more or less than the interface `Bootstrap`']
			],
			// an op that returns anything, and a file named from here
			[mutate(bootstrap, 'readonly pid: number;', 'readonly pid: any;'), ['bootstrap.d.ts any']],
			[`/// <reference path="./other.d.ts" />\n${bootstrap}`, ['bootstrap.d.ts references a file']]
		] satisfies Array<[string, Array<string>]>) {
			assert.deepEqual(find(mutated), found);
		}
		// a member whose type is written elsewhere, where this reads none of its members
		const named = 'interface Bootstrap {\n\treadonly core: Core;\n\treadonly primordials: {};\n}\n';
		assert.include(find(named), "Bootstrap.core isn't an object's type written in place");
	});

	test('a member of an intrinsic is found', () => {
		for (const [mutated, found] of [
			[
				mutate(intrinsics, 'interface Array<T> {}', 'interface Array<T> {\n\tlength: number;\n}'),
				['intrinsics.d.ts Array']
			],
			[
				mutate(
					intrinsics,
					'\tnew (message?: string): Error;\n',
					'\tnew (message?: string, options?: object): Error;\n'
				),
				['intrinsics.d.ts ErrorConstructor']
			],
			[
				mutate(
					intrinsics,
					'\tnew (message?: string): Error;\n',
					'\tnew (message?: string): Error;\n\tcaptureStackTrace(target: object): void;\n'
				),
				['intrinsics.d.ts ErrorConstructor']
			],
			[
				mutate(intrinsics, 'interface String {}', 'interface String extends Object {}'),
				['intrinsics.d.ts String extends']
			],
			// the one member allowed, of another interface
			[
				mutate(
					intrinsics,
					'interface Array<T> {}',
					'interface Array<T> {\n\tnew (message?: string): Error;\n}'
				),
				['intrinsics.d.ts Array']
			],
			// a value, which an implementation could call with no member read, and a type
			[
				`${intrinsics}declare function structuredClone<T>(value: T): T;\n`,
				['intrinsics.d.ts: declare function structuredClone<T>(value: T): T;']
			],
			[
				`${intrinsics}declare const JSON: { stringify(value: object): string };\n`,
				['intrinsics.d.ts: declare const JSON: { stringify(value: object): string };']
			],
			[
				`${intrinsics}type Loose = any;\n`,
				['intrinsics.d.ts any', 'intrinsics.d.ts: type Loose = any;']
			],
			[`/// <reference lib="es2024" />\n${intrinsics}`, ['intrinsics.d.ts references a file']]
		] satisfies Array<[string, Array<string>]>) {
			assert.deepEqual(find(bootstrap, mutated), found);
		}
	});
});
