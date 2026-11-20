import { describe, thrown } from '../describe.js';

/** What `call` throws, its class and message, or `null` when it returns. */
const refusal = (/** @type {() => unknown} */ call) => {
	try {
		call();
		return null;
	} catch (error) {
		const { constructor, message } = /** @type {any} */ (error);
		return [constructor?.name ?? typeof error, message];
	}
};

/** `bytes` in hex. */
const hex = (/** @type {Uint8Array} */ bytes) => {
	let text = '';
	for (const byte of bytes) text += byte.toString(16).padStart(2, '0');
	return text;
};

/**
 * A format's result as the case prints it: an error whole, and formatted code by its length,
 * whether it's the source as given, and its BLAKE3 digest, so the expectation holds what it
 * formatted to without holding the text.
 * @param {any} result
 * @param {string} source
 * @param {(data: Uint8Array) => Uint8Array} hash
 */
const formatted = (result, source, hash) =>
	result.ok
		? {
				keys: Object.keys(result),
				length: result.code.length,
				unchanged: result.code === source,
				digest: hex(hash(new TextEncoder().encode(result.code)))
			}
		: result;

/** Gro's own files, copied as the fixtures' `tsv/` inputs, each with its language. */
const OWN_FILES = [
	['tsv/format_file.ts.txt', 'typescript'],
	['tsv/dist_rewrite_imports.ts.txt', 'typescript'],
	['tsv/layout.svelte.txt', 'svelte'],
	['tsv/some.css.txt', 'css']
];

/** Sources for each language that format, change, or fail, by name. */
const SOURCES = {
	typescript: {
		unformatted: 'const  x:number=1;let y = {a:1,b:[1,2,3]}\nexport  function f(a,b){return a+b}',
		formatted: 'const x = 1;\n',
		empty: '',
		whitespace: '  \n\t\n',
		crlf: 'let a = 1;\r\nlet b = 2;\rlet c = 3;\n',
		bom: '\uFEFFlet a=1',
		multibyte: "const s = 'café 😀';let  t=s",
		sloppy: 'with (a) b;\nlet x = 010;',
		module_only: "import x from 'y';\nawait x;",
		syntax_error: 'let x = ;',
		multibyte_error: "const s = '😀';\nlet = ;",
		unclosed: 'function f() {\n\treturn [1, 2',
		jsx_like: 'const a = <T>(b: T) => b;'
	},
	svelte: {
		unformatted: '<script>let  x=1</script>\n<p   class="a">{ x }</p>',
		formatted: '<p>{x}</p>\n',
		empty: '',
		bom: '\uFEFF<p>x</p>',
		crlf: '<p>a</p>\r\n<p>b</p>\r\n',
		multibyte: '<p>café 😀 {x}</p>',
		syntax_error: '<p>{</p>',
		script_error: '<script lang="ts">let x: = 1;</script>',
		refused: '<div // a comment\rclass="x"\n></div>',
		style: '<style>a{color:red}</style>\n<a href="/">x</a>'
	},
	css: {
		unformatted: 'a{color:RED;margin:0 0 0 0}b>c{x:y}',
		formatted: 'a {\n\tcolor: red;\n}\n',
		empty: '',
		bom: '\uFEFFa{b:c}',
		crlf: 'a {\r\n\tb: c;\r\n}\r\n',
		multibyte: 'a::after{content:"😀 é"}',
		syntax_error: 'a { color: red;',
		bad_selector: '{}'
	}
};

/** Options each function is given: a value, and what it is. */
const OPTIONS = () => {
	/** @type {Array<[string, unknown]>} */
	const options = [
		['undefined', undefined],
		['null', null],
		['an empty object', {}],
		['a null-prototype empty object', Object.create(null)],
		['a number', 1],
		['a string', 'module'],
		['a boolean', true],
		['a symbol', Symbol('x')],
		['a bigint', 10n],
		['an array', []],
		['a function', () => {}],
		['a proxy', new Proxy({}, {})],
		['a boxed string', new String('x')],
		['a date', new Date(0)],
		['a map', new Map([['sourceType', 'script']])],
		['sourceType undefined', { sourceType: undefined }],
		['sourceType module', { sourceType: 'module' }],
		['sourceType script', { sourceType: 'script' }],
		['sourceType sloppy', { sourceType: 'sloppy' }],
		['sourceType empty', { sourceType: '' }],
		['sourceType a long string', { sourceType: 'm'.repeat(50) }],
		['sourceType a number', { sourceType: 1.5 }],
		['sourceType negative zero', { sourceType: -0 }],
		['sourceType NaN', { sourceType: NaN }],
		['sourceType a large number', { sourceType: 1e21 }],
		['sourceType false', { sourceType: false }],
		['sourceType null', { sourceType: null }],
		['sourceType an object', { sourceType: {} }],
		['sourceType a function', { sourceType: () => {} }],
		['sourceType a symbol', { sourceType: Symbol('s') }],
		['sourceType a bigint', { sourceType: 1n }],
		['sourceType a boxed string', { sourceType: new String('module') }],
		[
			'sourceType a getter',
			{
				get sourceType() {
					return 'module';
				}
			}
		],
		['an unknown key', { locations: true }],
		['an unknown key with sourceType after', { locatons: true, sourceType: 'module' }],
		['sourceType before an unknown key', { sourceType: 'module', x: 1 }],
		['an unknown key undefined', { x: undefined }],
		['an unknown key quoted', { 'it\'s "q"\\': 1 }],
		['an unknown key with controls', { 'a\nb\tc\u0001\u007f\u2028\u200e': 1 }],
		['an unknown key with a lone surrogate', { ['a\uD800b']: 1 }],
		['an unknown key long', { ['k'.repeat(45)]: 1 }],
		['an unknown key cut at a pair', { ['k'.repeat(39) + '😀x']: 1 }],
		['an unknown index key', { 0: 1 }],
		['keys in order', { 2: 1, 1: 1, sourceType: 'module' }],
		['a symbol key alone', { [Symbol('s')]: 1 }],
		[
			'a non-enumerable unknown key',
			Object.defineProperty({}, 'x', { value: 1, enumerable: false })
		],
		['an inherited sourceType', Object.create({ sourceType: 'sloppy' })],
		['an inherited unknown key', Object.create({ x: 1 })],
		['a frozen object', Object.freeze({ sourceType: 'script' })]
	];
	return options;
};

/** Sources each function is given that aren't one: a value, and what it is. */
const NOT_SOURCES = [
	['undefined', undefined],
	['null', null],
	['a number', 1],
	['a boolean', false],
	['a symbol', Symbol('x')],
	['a bigint', 1n],
	['an object', {}],
	['an array', ['x']],
	['a function', () => 'x'],
	['a boxed string', new String('let x = 1')],
	['a proxy', new Proxy({}, {})],
	['a lone high surrogate', 'let x = "\uD800";'],
	['a lone low surrogate', 'ok\uDC00'],
	['a reversed pair', '\uDC00\uD800'],
	['a lone surrogate after a pair', '😀\uD800']
];

/** A source nested `depth` deep in each language, past tsv's depth limits. */
const DEEP = {
	typescript: (/** @type {number} */ depth) => `${'['.repeat(depth)}${']'.repeat(depth)}`,
	typescript_chain: (/** @type {number} */ depth) => `x${'.a'.repeat(depth)};`,
	typescript_binary: (/** @type {number} */ depth) => `${'1+'.repeat(depth)}1;`,
	svelte: (/** @type {number} */ depth) => `${'<div>'.repeat(depth)}${'</div>'.repeat(depth)}`,
	svelte_blocks: (/** @type {number} */ depth) =>
		`${'{#if a}'.repeat(depth)}${'{/if}'.repeat(depth)}`,
	css: (/** @type {number} */ depth) => `${'a{'.repeat(depth)}${'}'.repeat(depth)}`
};

/** Import sources, TypeScript's and Svelte's, by name. */
const IMPORTS = {
	typescript: {
		static:
			"import a from './a.ts';\nimport { b, c as d } from './b.ts';\nimport * as e from 'e';\nimport 'side';",
		re_exports: "export { a } from './a.ts';\nexport * from './b.ts';\nexport * as c from 'c';",
		type_only:
			"import type { A } from './a.ts';\nimport { type B, type C } from './b.ts';\nimport { type D, e } from './d.ts';\nexport type { F } from './f.ts';",
		dynamic:
			"const a = import('./a.ts');\nconst b = import(`./b.ts`);\nconst c = import(name);\nconst d = import(`./${name}.ts`);\nimport('./e.ts').then(() => import('./f.ts'));",
		equals: "import a = require('./a');\nexport import b = require('b');",
		attributes:
			"import data from './data.json' with { type: 'json' };\nconst x = import('./x.json', { with: { type: 'json' } });",
		phases: "import source wasm from './a.wasm';\nimport defer * as ns from './b.ts';",
		escapes: "import a from './\\u0061.ts';\nimport b from '\\x62';\nconst c = import(`c\\\nd`);",
		multibyte: "const s = 'é😀';\nimport a from './é😀.ts';",
		bom: "\uFEFFimport a from './a.ts';",
		crlf: "import a from './a.ts';\r\nimport b from './b.ts';\r\n",
		script: 'var a = 1;\nwith (a) {}',
		type_positions: "type A = typeof import('./a.ts');\nlet b: import('./b.ts').B;",
		top_level_await: 'await 1;',
		empty: '',
		syntax_error: "import a from './a.ts'\nlet = ;",
		module_only_as_script: "import a from 'a';"
	},
	svelte: {
		instance:
			"<script lang=\"ts\">\n\timport A from './A.svelte';\n\timport type { B } from './b.ts';\n\tconst c = import('./c.ts');\n</script>\n<A />",
		both_scripts:
			"<script module>\n\texport const m = 1;\n\timport x from './x.ts';\n</script>\n<script>\n\timport y from './y.ts';\n</script>",
		markup_only: '<p>{x}</p>{#await import("./z.ts")}{/await}',
		bom: "\uFEFF<script>import a from './é.ts';</script>",
		multibyte: "<p>😀</p><script>import a from './a.ts';</script>",
		syntax_error: '<script>import a from;</script>',
		markup_error: '<p>{</p>',
		empty: ''
	}
};

/**
 * The scenarios of the tsv module, by name.
 * @type {Record<string, (ns: any, load: (name: string) => Promise<any>) => unknown>}
 */
const SCENARIOS = {
	// the namespace, each function, a stack, its prototype, and each method
	shape: (ns) => {
		const names = [
			'format_typescript',
			'format_svelte',
			'format_css',
			'parse_imports_typescript',
			'parse_imports_svelte',
			'create_ignore_stack'
		];
		const stack = ns.create_ignore_stack();
		const prototype = Object.getPrototypeOf(stack);
		const methods = [
			'push_gitignore',
			'pop_gitignore',
			'push_formatignore',
			'push_prettierignore',
			'pop_override',
			'is_ignored'
		];
		const result = ns.format_typescript('let x');
		const failure = ns.format_typescript('let =');
		const imports = ns.parse_imports_typescript("import a from 'a';");
		return {
			namespace: describe(ns),
			functions: names.map((name) => ({
				name,
				shape: describe(ns[name]),
				name_of: ns[name].name,
				length: ns[name].length
			})),
			stack: describe(stack),
			prototype: describe(prototype),
			prototype_prototype: Object.getPrototypeOf(prototype),
			methods: methods.map((name) => ({
				name,
				shape: describe(prototype[name]),
				length: prototype[name].length,
				name_of: prototype[name].name
			})),
			one_prototype: Object.getPrototypeOf(ns.create_ignore_stack()) === prototype,
			another_each_call: ns.create_ignore_stack() !== stack,
			own_keys: Reflect.ownKeys(stack),
			json: JSON.stringify(stack),
			thenable: 'then' in stack,
			result: describe(result),
			failure: describe(failure),
			error: describe(failure.error),
			loc: describe(failure.error.loc),
			point: describe(failure.error.loc.start),
			imports: describe(imports),
			records: describe(imports.imports),
			record: describe(imports.imports[0]),
			specifier: describe(imports.imports[0].specifier),
			another_result_each_call: ns.format_typescript('let x') !== result,
			new_format: thrown(() => new ns.format_typescript('x')),
			new_factory: thrown(() => new ns.create_ignore_stack()),
			new_method: thrown(() => new stack.is_ignored('a', false)),
			property_write: thrown(() => {
				stack.extra = 1;
			}),
			method_write: thrown(() => {
				prototype.is_ignored = () => true;
			}),
			function_write: thrown(() => {
				ns.format_css.stash = 'x';
			}),
			namespace_write: thrown(() => {
				ns.format_css = null;
			})
		};
	},
	// Gro's own files and sources of each language that format, change, or fail
	format: async (ns, load) => {
		const { fs } = await load('fs');
		const { hash } = await load('blake3');
		const format = {
			typescript: ns.format_typescript,
			svelte: ns.format_svelte,
			css: ns.format_css
		};
		/** @type {Record<string, unknown>} */
		const own = {};
		for (const [path, language] of OWN_FILES) {
			const source = await fs.project.read_text(path);
			const result = format[language](source);
			own[path] = formatted(result, source, hash);
			// formatting the formatted output again changes nothing
			own[`${path} again`] = result.ok ? format[language](result.code).code === result.code : null;
		}
		/** @type {Record<string, Record<string, unknown>>} */
		const sources = {};
		for (const [language, by_name] of Object.entries(SOURCES)) {
			sources[language] = {};
			for (const [name, source] of Object.entries(by_name)) {
				sources[language][name] = formatted(format[language](source), source, hash);
			}
		}
		const goals = {};
		for (const name of ['sloppy', 'module_only', 'unformatted', 'syntax_error']) {
			const source = SOURCES.typescript[name];
			goals[name] = ['script', 'module', undefined].map((sourceType) =>
				formatted(ns.format_typescript(source, { sourceType }), source, hash)
			);
		}
		return { own, sources, goals };
	},
	// what options each function takes, and refuses
	options: (ns) => {
		const functions = [
			'format_typescript',
			'format_svelte',
			'format_css',
			'parse_imports_typescript',
			'parse_imports_svelte'
		];
		const source = { format_css: 'a{}', format_svelte: '<p></p>', parse_imports_svelte: '<p></p>' };
		return functions.map((name) => [
			name,
			OPTIONS().map(([what, options]) => {
				let result;
				try {
					const returned = ns[name](source[name] ?? 'let x = 1;', options);
					result = returned.ok ? 'ok' : returned.error.message;
				} catch (error) {
					result = [
						/** @type {any} */ (error).constructor.name,
						/** @type {any} */ (error).message
					];
				}
				return [what, result];
			})
		]);
	},
	// what counts as a source, checked before the options
	sources: (ns) => {
		const functions = [
			'format_typescript',
			'format_svelte',
			'format_css',
			'parse_imports_typescript',
			'parse_imports_svelte'
		];
		return functions.map((name) => [
			name,
			NOT_SOURCES.map(([what, source]) => [what, refusal(() => ns[name](source))]),
			// a source is refused before its options are read
			refusal(() => ns[name](1, 1)),
			refusal(() => ns[name](undefined, { x: 1 }))
		]);
	},
	// input nested past tsv's depth limits, a result and never a crash
	deep: (ns) => {
		const call = {
			typescript: ns.format_typescript,
			typescript_chain: ns.format_typescript,
			typescript_binary: ns.format_typescript,
			svelte: ns.format_svelte,
			svelte_blocks: ns.format_svelte,
			css: ns.format_css
		};
		const imports = {
			typescript: ns.parse_imports_typescript,
			typescript_chain: ns.parse_imports_typescript,
			typescript_binary: ns.parse_imports_typescript,
			svelte: ns.parse_imports_svelte,
			svelte_blocks: ns.parse_imports_svelte
		};
		/** @type {Record<string, unknown>} */
		const out = {};
		for (const [name, make] of Object.entries(DEEP)) {
			for (const depth of [100, 300, 10000, 100000]) {
				const source = make(depth);
				const result = call[name](source);
				out[`format ${name} ${depth}`] = result.ok ? `ok ${result.code.length}` : result.error;
				const parse = imports[name];
				if (parse) {
					const parsed = parse(source);
					out[`imports ${name} ${depth}`] = parsed.ok
						? `ok ${parsed.imports.length}`
						: parsed.error;
				}
			}
		}
		return out;
	},
	// every kind of request, its spans, and a source that doesn't parse
	imports: (ns) => {
		/** @type {Record<string, unknown>} */
		const out = {};
		for (const [name, source] of Object.entries(IMPORTS.typescript)) {
			out[`typescript ${name}`] = ns.parse_imports_typescript(source);
			out[`typescript ${name} as a module`] = ns.parse_imports_typescript(source, {
				sourceType: 'module'
			});
			out[`typescript ${name} as a script`] = ns.parse_imports_typescript(source, {
				sourceType: 'script'
			});
		}
		for (const [name, source] of Object.entries(IMPORTS.svelte)) {
			out[`svelte ${name}`] = ns.parse_imports_svelte(source);
		}
		// gro's own files
		out['svelte as typescript'] = ns.parse_imports_typescript(IMPORTS.svelte.instance);
		return out;
	},
	// a stack's layers, nested and negated, overrides, pops, and what's a directory
	ignore: (ns) => {
		const paths = [
			['a.ts', false],
			['dist', true],
			['dist', false],
			['dist/x.js', false],
			['dist/keep.js', false],
			['src/a.gen.ts', false],
			['src/b/a.gen.ts', false],
			['src/b/keep.gen.ts', false],
			['src/b', true],
			['logs/x.log', false],
			['x.log', false],
			['important.log', false],
			['nested/build', true],
			['build', true],
			['build/out', false],
			['a/b/c.txt', false],
			['a/b/d.md', false],
			['a/x.md', false],
			['', false],
			['/abs', false],
			['./a.ts', false],
			['café/😀.ts', false],
			['space name.txt', false],
			['back\\slash', false],
			['trailing/', true]
		];
		const check = (/** @type {any} */ stack) =>
			paths.map(
				([path, is_dir]) => `${path}${is_dir ? '/' : ''} ${stack.is_ignored(path, is_dir)}`
			);
		const stack = ns.create_ignore_stack();
		const empty = check(stack);
		stack.push_gitignore(
			'',
			'dist/\n*.log\n!important.log\n/build\n# a comment\n\\#literal\nspace\\ name.txt\ncafé/\n'
		);
		const root = check(stack);
		stack.push_gitignore('src', '*.gen.ts\n!keep.gen.ts\n');
		stack.push_gitignore('a/b', '*.txt\n');
		const nested = check(stack);
		stack.push_formatignore('', '!dist/keep.js\n*.md\n');
		const formatignore = check(stack);
		stack.push_prettierignore('a', '!b/d.md\n');
		const prettierignore = check(stack);
		stack.pop_override();
		stack.pop_override();
		const popped_overrides = check(stack);
		stack.pop_override();
		stack.pop_gitignore();
		const popped_gitignore = check(stack);
		stack.pop_gitignore();
		stack.pop_gitignore();
		stack.pop_gitignore();
		const popped_all = check(stack);
		const crlf = ns.create_ignore_stack();
		crlf.push_gitignore('', '\uFEFFdist/\r\n*.log\r\n');
		const lone = ns.create_ignore_stack();
		lone.push_gitignore('', 'a\uD800\n');
		const others = {
			crlf: check(crlf),
			lone_surrogate: lone.is_ignored('a\uD800', false),
			lone_surrogate_replaced: lone.is_ignored('a\uFFFD', false)
		};
		// two stacks are apart
		const apart = ns.create_ignore_stack();
		apart.push_gitignore('', '*\n');
		return {
			empty,
			root,
			nested,
			formatignore,
			prettierignore,
			popped_overrides,
			popped_gitignore,
			popped_all,
			others,
			apart: [apart.is_ignored('x', false), stack.is_ignored('x', false)]
		};
	},
	// a method called on what isn't a stack, refused before its arguments are read, and
	// arguments that aren't strings and booleans
	receiver: (ns) => {
		const stack = ns.create_ignore_stack();
		const prototype = Object.getPrototypeOf(stack);
		/** @type {Array<[string, unknown]>} */
		const receivers = [
			['undefined', undefined],
			['an object', {}],
			['a look-alike', Object.create(prototype)],
			['a proxy of a stack', new Proxy(stack, {})],
			['a copy', { ...stack }],
			['a function', () => {}]
		];
		const calls = {
			push_gitignore: ['', ''],
			pop_gitignore: [],
			push_formatignore: ['', ''],
			push_prettierignore: ['', ''],
			pop_override: [],
			is_ignored: ['a', false]
		};
		/** @type {Array<[string, unknown]>} */
		const not_strings = [
			['undefined', undefined],
			['null', null],
			['a number', 1],
			['a boxed string', new String('x')],
			['an array', ['x']],
			['an object', { toString: () => 'x' }]
		];
		const borrowed = ns.create_ignore_stack();
		stack.push_gitignore.call(borrowed, '', '*\n');
		return {
			receivers: Object.entries(calls).map(([name, args]) => [
				name,
				receivers.map(([what, receiver]) => [
					what,
					refusal(() => prototype[name].call(receiver, ...args))
				]),
				// the receiver is checked before the arguments
				refusal(() => prototype[name].call({}, 1, 1))
			]),
			layers: ['push_gitignore', 'push_formatignore', 'push_prettierignore'].map((name) => [
				name,
				not_strings.map(([what, value]) => [
					what,
					refusal(() => stack[name](value, '')),
					refusal(() => stack[name]('', value))
				]),
				refusal(() => stack[name]('')),
				refusal(() => stack[name]('', '', 'extra'))
			]),
			paths: not_strings.map(([what, value]) => [
				what,
				refusal(() => stack.is_ignored(value, false))
			]),
			dirs: [
				['undefined', undefined],
				['a number', 1],
				['a string', 'true'],
				['a boxed boolean', new Boolean(true)],
				['null', null]
			].map(([what, value]) => [what, refusal(() => stack.is_ignored('a', value))]),
			// a refused push leaves the stack as it was
			after_refusals: stack.is_ignored('x', false),
			// a stack's methods borrowed by another stack act on that one
			borrowed: [borrowed.is_ignored('x', false), stack.is_ignored('x', false)]
		};
	}
};

/**
 * @param {any} ns
 * @param {Array<string>} rest - the scenario's name
 * @param {(value: unknown) => void} _print
 * @param {(name: string) => Promise<any>} load
 */
export default (ns, [scenario], _print, load) => {
	const run = SCENARIOS[/** @type {string} */ (scenario)];
	if (!run) throw new Error(`no tsv scenario named ${scenario}`);
	return run(ns, load);
};
