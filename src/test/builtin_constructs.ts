import { assert } from 'vitest';
import { join } from 'node:path';
import ts from 'typescript';

import { BUILTIN_DIR, type BuiltinRow } from './builtin_test_helpers.ts';

/*

The static check of a builtin module's implementation
(`crates/gro_runtime/js/builtin/<specifier>`), one pass over its own program
(`builtin/tsconfig.json`): the program's diagnostics, and a walk of the implementation's
syntax, with the program's types, for what the program accepts and that still runs code or
reads state a program can reach.

The program has no library but `intrinsics.d.ts`, so a global or a member of an intrinsic is a
type error. What it can't refuse, the walk does, by one rule and a list:

- a member is read only of what the bootstrap object gave: the outer level's parameter, a
  `const` bound from it by an object pattern or a plain initializer, and a member of one of
  those. Anything else an implementation holds is a value user code made or can reach (an
  argument, the scope, a result, `this`, a literal whose prototype is the realm's), where a
  read runs a getter or a proxy's trap, or falls through to `Object.prototype` when the member
  is absent, so a member declared optional is refused even of the bootstrap object. A private
  name is the exception: it's read of the object alone, throws on one that lacks it, and
  `#name in value` asks which, so a class's brand is checked with no member read. No property
  is written at all, since a write of one an object lacks reaches a setter on its prototype.
- each construct that calls a protocol user code can answer: iteration (`for`-`of`, a spread,
  an array pattern or a destructuring assignment, a generator), a spread of an object that
  isn't a literal (its keys and getters), a string or a number made of a value that isn't
  primitive (a template, `+`, a comparison, arithmetic, a computed key), `await` and `async`
  (`then`), `instanceof` (`Symbol.hasInstance`), `in` of a name that isn't private and
  `for`-`in` (the prototype chain, a proxy's traps), a tagged template, and `extends` (the
  superclass's prototype).
- what leaves the type check: `any`, a type assertion but `as const`, a non-null assertion, a
  `@ts-` directive, `globalThis` (the one global the compiler declares itself), `arguments`,
  a `declare` (which names a global the program lacks), an overload and a type predicate (each
  gives a value a type no check made), and a write of the outer level's parameter.

A parameter's declared type is a claim about a caller, which no static check tests: a
function that makes a string of a parameter declared a string is accepted here, and what it
does with a value that isn't one is the runtime tests' to show.

*/

const CONFIG_PATH = join(BUILTIN_DIR, 'tsconfig.json');

/** The declaration of what an implementation is given, `Bootstrap`. */
export const BOOTSTRAP_DECLARATIONS_PATH = join(BUILTIN_DIR, 'bootstrap.d.ts');

/** The only global declarations an implementation is type-checked with. */
export const INTRINSICS_DECLARATIONS_PATH = join(BUILTIN_DIR, 'intrinsics.d.ts');

/**
 * The implementations' program's options and files, as its tsconfig has them.
 */
export const read_builtin_config = (): ts.ParsedCommandLine => {
	const config = ts.readConfigFile(CONFIG_PATH, ts.sys.readFile);
	assert.ok(!config.error, 'the builtin tsconfig reads');
	return ts.parseJsonConfigFileContent(config.config, ts.sys, BUILTIN_DIR);
};

// the files every check reads from disk, parsed once
const source_files: Map<string, ts.SourceFile | undefined> = new Map();

export interface ImplementationAnalysis {
	/** The program's diagnostics, each as text. */
	diagnostics: Array<string>;
	/** The names of what the walk refuses, each once, sorted. */
	refused: Array<string>;
}

/**
 * Type-checks the implementations' program, the files its tsconfig includes, and walks one
 * implementation's syntax.
 * @param row - the row whose implementation is walked
 * @param implementation - the source in place of that row's file, when given
 * @param declarations - the source in place of `bootstrap.d.ts`, when given
 */
export const analyze_implementation = (
	row: BuiltinRow,
	implementation?: string,
	declarations?: string
): ImplementationAnalysis => {
	const { options, fileNames } = read_builtin_config();
	const host = ts.createCompilerHost(options);
	const get_source_file = host.getSourceFile.bind(host);
	host.getSourceFile = (file_name, language_version, ...rest) => {
		if (implementation !== undefined && file_name === row.path) {
			return ts.createSourceFile(file_name, implementation, language_version, true);
		}
		if (declarations !== undefined && file_name === BOOTSTRAP_DECLARATIONS_PATH) {
			return ts.createSourceFile(file_name, declarations, language_version, true);
		}
		if (!source_files.has(file_name)) {
			source_files.set(file_name, get_source_file(file_name, language_version, ...rest));
		}
		return source_files.get(file_name);
	};
	const program = ts.createProgram(fileNames, options, host);
	const diagnostics = ts
		.getPreEmitDiagnostics(program)
		.map((d) => ts.flattenDiagnosticMessageText(d.messageText, '\n'));
	const source = program.getSourceFile(row.path);
	assert.ok(source, `${row.path} is in the program`);
	const refused = new Set(find_refused_constructs(source, program.getTypeChecker()));
	return { diagnostics, refused: Array.from(refused).sort() };
};

const PRIMITIVE =
	ts.TypeFlags.StringLike |
	ts.TypeFlags.NumberLike |
	ts.TypeFlags.BooleanLike |
	ts.TypeFlags.BigIntLike |
	ts.TypeFlags.ESSymbolLike |
	ts.TypeFlags.Null |
	ts.TypeFlags.Undefined |
	ts.TypeFlags.Void |
	ts.TypeFlags.Never;

const is_primitive = (type: ts.Type): boolean =>
	type.isUnion() ? type.types.every(is_primitive) : (type.flags & PRIMITIVE) !== 0;

/** The binary operators that make a string or a number of an operand that isn't one. */
const COERCING: ReadonlySet<ts.SyntaxKind> = new Set([
	ts.SyntaxKind.PlusToken,
	ts.SyntaxKind.MinusToken,
	ts.SyntaxKind.AsteriskToken,
	ts.SyntaxKind.AsteriskAsteriskToken,
	ts.SyntaxKind.SlashToken,
	ts.SyntaxKind.PercentToken,
	ts.SyntaxKind.LessThanLessThanToken,
	ts.SyntaxKind.GreaterThanGreaterThanToken,
	ts.SyntaxKind.GreaterThanGreaterThanGreaterThanToken,
	ts.SyntaxKind.AmpersandToken,
	ts.SyntaxKind.BarToken,
	ts.SyntaxKind.CaretToken,
	ts.SyntaxKind.EqualsEqualsToken,
	ts.SyntaxKind.ExclamationEqualsToken,
	ts.SyntaxKind.LessThanToken,
	ts.SyntaxKind.LessThanEqualsToken,
	ts.SyntaxKind.GreaterThanToken,
	ts.SyntaxKind.GreaterThanEqualsToken,
	ts.SyntaxKind.PlusEqualsToken,
	ts.SyntaxKind.MinusEqualsToken,
	ts.SyntaxKind.AsteriskEqualsToken,
	ts.SyntaxKind.AsteriskAsteriskEqualsToken,
	ts.SyntaxKind.SlashEqualsToken,
	ts.SyntaxKind.PercentEqualsToken,
	ts.SyntaxKind.LessThanLessThanEqualsToken,
	ts.SyntaxKind.GreaterThanGreaterThanEqualsToken,
	ts.SyntaxKind.GreaterThanGreaterThanGreaterThanEqualsToken,
	ts.SyntaxKind.AmpersandEqualsToken,
	ts.SyntaxKind.BarEqualsToken,
	ts.SyntaxKind.CaretEqualsToken
]);

const COERCING_UNARY: ReadonlySet<ts.SyntaxKind> = new Set([
	ts.SyntaxKind.PlusToken,
	ts.SyntaxKind.MinusToken,
	ts.SyntaxKind.TildeToken,
	ts.SyntaxKind.PlusPlusToken,
	ts.SyntaxKind.MinusMinusToken
]);

const is_assignment = (kind: ts.SyntaxKind): boolean =>
	kind >= ts.SyntaxKind.FirstAssignment && kind <= ts.SyntaxKind.LastAssignment;

const skip_parentheses = (node: ts.Expression): ts.Expression =>
	ts.isParenthesizedExpression(node) ? skip_parentheses(node.expression) : node;

/** Whether `node` reads or writes a member by a name that isn't private. */
const is_public_member = (node: ts.Node): boolean =>
	ts.isElementAccessExpression(node) ||
	(ts.isPropertyAccessExpression(node) && !ts.isPrivateIdentifier(node.name));

const is_const = (declaration: ts.VariableDeclaration): boolean =>
	ts.isVariableDeclarationList(declaration.parent) &&
	(declaration.parent.flags & ts.NodeFlags.Const) !== 0;

/**
 * The outer level of an implementation: the function its one statement is.
 */
const find_outer_level = (source: ts.SourceFile): ts.FunctionExpression | undefined => {
	const [statement, ...others] = source.statements;
	if (statement === undefined || others.length > 0 || !ts.isExpressionStatement(statement)) {
		return undefined;
	}
	const expression = skip_parentheses(statement.expression);
	return ts.isFunctionExpression(expression) ? expression : undefined;
};

/**
 * What `source`, an implementation, holds that its program accepts and that runs code or
 * reads state a program can reach, or that leaves the type check, each named, in the order
 * found.
 */
const find_refused_constructs = (source: ts.SourceFile, checker: ts.TypeChecker): Array<string> => {
	const found: Array<string> = [];
	if (/@ts-(?:ignore|expect-error|nocheck)/.test(source.text)) found.push('@ts- directive');
	const outer = find_outer_level(source);
	if (outer === undefined) found.push('not one function');

	const primitive = (node: ts.Node): boolean => is_primitive(checker.getTypeAtLocation(node));

	/**
	 * Whether the variable `declaration` declares, by an object pattern or by its name, is
	 * bound from what the bootstrap object gave, a `const` so it stays that.
	 */
	const is_given_variable = (declaration: ts.VariableDeclaration): boolean =>
		is_const(declaration) &&
		declaration.initializer !== undefined &&
		is_given(declaration.initializer);

	/**
	 * Whether `expression` is what the bootstrap object gave: the outer level's parameter, a
	 * `const` bound from it, or a member of one of those.
	 */
	const is_given = (expression: ts.Expression): boolean => {
		const given = skip_parentheses(expression);
		if (ts.isPropertyAccessExpression(given)) {
			return given.questionDotToken === undefined && is_given(given.expression);
		}
		if (!ts.isIdentifier(given)) return false;
		let declaration: ts.Node | undefined = checker.getSymbolAtLocation(given)?.valueDeclaration;
		if (declaration === undefined) return false;
		if (ts.isParameter(declaration)) {
			return declaration.parent === outer && ts.isIdentifier(declaration.name);
		}
		// a name an object pattern binds, at any depth of the pattern
		while (ts.isBindingElement(declaration) && ts.isObjectBindingPattern(declaration.parent)) {
			declaration = declaration.parent.parent;
		}
		return ts.isVariableDeclaration(declaration) && is_given_variable(declaration);
	};

	/** Whether the member `name` names is declared optional, so it can be absent. */
	const is_optional = (name: ts.Node): boolean => {
		const symbol = checker.getSymbolAtLocation(name);
		return symbol !== undefined && (symbol.flags & ts.SymbolFlags.Optional) !== 0;
	};

	/** Whether the object pattern `pattern` takes its members from what the bootstrap gave. */
	const is_given_pattern = (pattern: ts.ObjectBindingPattern): boolean => {
		let declaration: ts.Node = pattern.parent;
		while (ts.isBindingElement(declaration) && ts.isObjectBindingPattern(declaration.parent)) {
			declaration = declaration.parent.parent;
		}
		return ts.isVariableDeclaration(declaration) && is_given_variable(declaration);
	};

	const visit = (node: ts.Node): void => {
		if (node.kind === ts.SyntaxKind.AnyKeyword) found.push('any');
		if (ts.isIdentifier(node) && node.text === 'globalThis') found.push('globalThis');
		if (ts.isIdentifier(node) && node.text === 'arguments') found.push('arguments');
		if (
			(ts.isAsExpression(node) || ts.isTypeAssertionExpression(node)) &&
			!ts.isConstTypeReference(node.type)
		) {
			found.push('type assertion');
		}
		if (ts.isNonNullExpression(node)) found.push('type assertion');
		if (
			ts.canHaveModifiers(node) &&
			ts.getModifiers(node)?.some((modifier) => modifier.kind === ts.SyntaxKind.DeclareKeyword)
		) {
			found.push('declare');
		}
		if (
			(ts.isFunctionDeclaration(node) || ts.isMethodDeclaration(node)) &&
			node.body === undefined
		) {
			found.push('overload');
		}
		if (ts.isTypePredicateNode(node)) found.push('type predicate');

		// a member, read
		if (ts.isElementAccessExpression(node)) found.push('member');
		if (ts.isPropertyAccessExpression(node) && !ts.isPrivateIdentifier(node.name)) {
			const given = node.questionDotToken === undefined && is_given(node.expression);
			if (!given || is_optional(node.name)) found.push('member');
		}
		if (ts.isObjectBindingPattern(node)) {
			if (!is_given_pattern(node)) found.push('member');
			for (const element of node.elements) {
				const name = element.propertyName ?? element.name;
				const plain = ts.isIdentifier(name) || ts.isObjectBindingPattern(name);
				// a default is what an absent member takes, and a computed key is a string made
				if (element.initializer !== undefined || !plain) found.push('member');
				if (ts.isIdentifier(name) && element.dotDotDotToken === undefined) {
					const from = checker.getTypeAtLocation(node);
					const member = checker.getPropertyOfType(from, name.text);
					if (member !== undefined && (member.flags & ts.SymbolFlags.Optional) !== 0) {
						found.push('member');
					}
				}
			}
		}

		// a property, written
		if (ts.isBinaryExpression(node) && is_assignment(node.operatorToken.kind)) {
			const target = skip_parentheses(node.left);
			if (is_public_member(target)) found.push('property write');
			if (ts.isIdentifier(target)) {
				const declaration = checker.getSymbolAtLocation(target)?.valueDeclaration;
				if (declaration !== undefined && ts.isParameter(declaration)) {
					if (declaration.parent === outer) found.push('parameter write');
				}
			}
			if (ts.isObjectLiteralExpression(target) || ts.isArrayLiteralExpression(target)) {
				found.push('destructuring assignment');
			}
		}
		if (
			(ts.isPrefixUnaryExpression(node) || ts.isPostfixUnaryExpression(node)) &&
			(node.operator === ts.SyntaxKind.PlusPlusToken ||
				node.operator === ts.SyntaxKind.MinusMinusToken) &&
			is_public_member(skip_parentheses(node.operand))
		) {
			found.push('property write');
		}
		if (ts.isDeleteExpression(node)) found.push('property write');

		// iteration
		if (ts.isForOfStatement(node)) found.push('for-of');
		if (ts.isSpreadElement(node)) found.push('spread');
		if (ts.isArrayBindingPattern(node)) found.push('array pattern');
		if (
			(ts.isFunctionDeclaration(node) ||
				ts.isFunctionExpression(node) ||
				ts.isMethodDeclaration(node)) &&
			node.asteriskToken !== undefined
		) {
			found.push('generator');
		}
		if (
			ts.isSpreadAssignment(node) &&
			!ts.isObjectLiteralExpression(skip_parentheses(node.expression))
		) {
			found.push('object spread');
		}

		// a string or a number made of a value
		if (ts.isBinaryExpression(node) && COERCING.has(node.operatorToken.kind)) {
			if (!primitive(node.left) || !primitive(node.right)) found.push('coercion');
		}
		if (
			(ts.isPrefixUnaryExpression(node) || ts.isPostfixUnaryExpression(node)) &&
			COERCING_UNARY.has(node.operator) &&
			!primitive(node.operand)
		) {
			found.push('coercion');
		}
		if (ts.isTemplateSpan(node) && !primitive(node.expression)) found.push('coercion');
		if (ts.isComputedPropertyName(node) && !primitive(node.expression)) found.push('coercion');

		// the other protocols
		if (ts.isAwaitExpression(node)) found.push('await');
		if (
			ts.isFunctionLike(node) &&
			ts.canHaveModifiers(node) &&
			ts.getModifiers(node)?.some((modifier) => modifier.kind === ts.SyntaxKind.AsyncKeyword)
		) {
			found.push('async');
		}
		if (ts.isBinaryExpression(node)) {
			if (node.operatorToken.kind === ts.SyntaxKind.InstanceOfKeyword) found.push('instanceof');
			// `#name in value` asks whether an object has a private name, of the object alone
			if (node.operatorToken.kind === ts.SyntaxKind.InKeyword) {
				if (!ts.isPrivateIdentifier(node.left)) found.push('in');
			}
		}
		if (ts.isForInStatement(node)) found.push('for-in');
		if (ts.isTaggedTemplateExpression(node)) found.push('tagged template');
		if (ts.isHeritageClause(node) && node.token === ts.SyntaxKind.ExtendsKeyword) {
			if (ts.isClassLike(node.parent)) found.push('extends');
		}
		ts.forEachChild(node, visit);
	};
	visit(source);
	return found;
};

/** A member of the bootstrap object an implementation may be given, as it's reviewed. */
export interface ReviewedMember {
	/** Its declared type, as `bootstrap.d.ts` writes it. */
	type: string;
	/** Why an implementation may use it after user code ran, and how. */
	why: string;
}

/**
 * What `bootstrap.d.ts` and `intrinsics.d.ts` declare that isn't reviewed, or declare
 * otherwise than reviewed, each named: the bootstrap object's members but `core` and
 * `primordials`, a member of `core` but `ops`, an op that isn't a host op of gro's, a
 * primordial that isn't in `primordials` with that type, a reviewed one that's gone, and a
 * member of an intrinsic's type but the one way an `Error` is made.
 * @param bootstrap - the text of `bootstrap.d.ts`
 * @param intrinsics - the text of `intrinsics.d.ts`
 * @param primordials - the reviewed primordials, by name
 */
export const find_unreviewed_declarations = (
	bootstrap: string,
	intrinsics: string,
	primordials: Record<string, ReviewedMember>
): Array<string> => {
	const found: Array<string> = [];
	const parse = (name: string, text: string): ts.SourceFile =>
		ts.createSourceFile(name, text, ts.ScriptTarget.ES2024, true);
	const to_text = (node: ts.Node): string => node.getText().replace(/\s+/g, ' ');

	/** The members of an interface or a type literal, each a property with a type, by name. */
	const read_members = (
		owner: string,
		members: ts.NodeArray<ts.TypeElement>
	): Map<string, ts.TypeNode> => {
		const read: Map<string, ts.TypeNode> = new Map();
		for (const member of members) {
			if (
				ts.isPropertySignature(member) &&
				ts.isIdentifier(member.name) &&
				member.type !== undefined &&
				member.questionToken === undefined
			) {
				read.set(member.name.text, member.type);
			} else {
				found.push(`${owner}: ${to_text(member)}`);
			}
		}
		return read;
	};

	/** The members of the type literal `type` is, by name, and none when it isn't one. */
	const read_literal = (owner: string, type: ts.TypeNode | undefined): Map<string, ts.TypeNode> => {
		if (type !== undefined && ts.isTypeLiteralNode(type)) return read_members(owner, type.members);
		found.push(`${owner} isn't an object's type written in place`);
		return new Map();
	};

	const declared = parse('bootstrap.d.ts', bootstrap);
	const interfaces = declared.statements.filter(ts.isInterfaceDeclaration);
	const [declaration] = interfaces;
	if (
		declaration === undefined ||
		declaration.name.text !== 'Bootstrap' ||
		declared.statements.length !== 1 ||
		declaration.heritageClauses !== undefined
	) {
		return ['bootstrap.d.ts declares more or less than the interface `Bootstrap`'];
	}
	const members = read_members('Bootstrap', declaration.members);
	for (const name of members.keys()) {
		if (name !== 'core' && name !== 'primordials') found.push(`Bootstrap.${name}`);
	}
	const core = read_literal('Bootstrap.core', members.get('core'));
	for (const name of core.keys()) {
		if (name !== 'ops') found.push(`core.${name}`);
	}
	for (const [name, type] of read_literal('core.ops', core.get('ops'))) {
		if (!name.startsWith('op_gro_') || !ts.isFunctionTypeNode(type)) found.push(`core.ops.${name}`);
	}
	const declared_primordials = read_literal('Bootstrap.primordials', members.get('primordials'));
	for (const [name, type] of declared_primordials) {
		const reviewed = primordials[name];
		if (reviewed === undefined) {
			found.push(`primordials.${name}`);
		} else if (to_text(type) !== reviewed.type) {
			found.push(`primordials.${name}: ${to_text(type)}`);
		}
	}
	for (const name of Object.keys(primordials)) {
		if (!declared_primordials.has(name)) found.push(`primordials.${name} is reviewed and gone`);
	}

	for (const statement of parse('intrinsics.d.ts', intrinsics).statements) {
		if (!ts.isInterfaceDeclaration(statement)) continue;
		const name = statement.name.text;
		const made =
			name === 'ErrorConstructor' &&
			statement.members.length === 1 &&
			to_text(statement.members[0]!) === 'new (message?: string): Error;';
		if (statement.members.length > 0 && !made) found.push(`intrinsics.d.ts ${name}`);
		if (statement.heritageClauses !== undefined) found.push(`intrinsics.d.ts ${name} extends`);
	}
	return found;
};
