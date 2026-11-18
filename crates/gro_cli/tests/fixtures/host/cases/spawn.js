import { describe, thrown } from '../describe.js';

// The spawn host module's handles, over the programs under `spawn/`: `bin/` holds a `#!` script
// (`tool`), a link to it (`link`), a file with ELF's magic (`elf`), a `#!` naming no interpreter
// (`bare`), which pins as a script, one without execute permission (`noexec`), one that's
// neither a script nor a binary (`text`), and a `.cmd`;
// `dir/` holds only a file without execute permission; `nm/node_modules/.bin`, a link to
// `shadow/.bin`, holds a program a `PATH` walk skips by that spelling; and `rel/` holds one only
// a relative `PATH` entry names, which the walk skips too. Each case's `PATH` names
// these by `{cwd}`, the fixtures' directory, and each path a value or an error names is shown
// with the project's path as `{project}`, since the checkout's path is the machine's. No program
// is started: a case makes handles and reads what they hold.

/** @type {string} */
let project = '';

/** `text` with the project's path as `{project}`. */
const relative = (/** @type {unknown} */ text) =>
	typeof text === 'string' ? text.split(project).join('{project}') : text;

/** What a thrown value is to a program, as data. */
const show_error = (/** @type {any} */ error) => {
	if (!(error instanceof Error)) return { not_an_error: typeof error };
	const { constructor, message, kind, program, cause } = error;
	return {
		class: constructor?.name,
		message: relative(message),
		kind,
		program,
		cause: cause instanceof Error ? relative(cause.message) : typeof cause
	};
};

/** A program handle as data: its name, and its path when it's the project's. */
const show_program = (/** @type {any} */ value) => {
	if (value === null || typeof value !== 'object') return value;
	const { name, path } = value;
	return {
		name,
		path: typeof path === 'string' && path.startsWith(project) ? relative(path) : typeof path
	};
};

/** What `call` returned or threw, as data. */
const outcome = (/** @type {() => unknown} */ call) => {
	try {
		return { value: show_program(call()) };
	} catch (error) {
		return { threw: show_error(error) };
	}
};

/** What a promise `call` returns settled to, as data, or what `call` threw. */
const settled = async (/** @type {() => Promise<unknown>} */ call) => {
	try {
		return { fulfilled: typeof (await call()) };
	} catch (error) {
		return { rejected: show_error(error) };
	}
};

/** Each of `names` asked for by `program` and `find` of `root`, with `options`. */
const ask_all = (/** @type {any} */ root, /** @type {Array<string>} */ names, options) =>
	Object.fromEntries(
		names.map((name) => [
			name,
			{
				program: outcome(() => root.program(name, options)),
				find: outcome(() => root.find(name, options))
			}
		])
	);

/** The names the cases ask for, found on `PATH`, beneath the project, or neither. */
const NAMES = [
	'tool',
	'elf',
	'shadowed',
	'relonly',
	'noexec_only',
	'missing',
	'run.cmd',
	'./spawn/bin/tool',
	'spawn/bin/link',
	'./spawn/bin/elf',
	'./spawn/bin/bare',
	'./spawn/bin/noexec',
	'./spawn/bin/text',
	'./spawn/bin/run.cmd',
	'./spawn/bin/RUN.CMD. ',
	'./spawn/bin/missing',
	'./spawn/bin/',
	'./spawn/bin/tool/',
	'./spawn/bin/tool/.',
	'spawn/../spawn/bin/tool',
	'sh'
];

/** The names a case asks for that only an unconfined run reaches: absolute, and climbing out. */
const OUTSIDE = ['/bin/sh', '../spawn/bin/tool', 'spawn/../../x'];

/**
 * @param {typeof import('../../../../../../src/lib/spawn.ts')} ns
 * @param {Array<string>} args
 */
export default async (ns, [scenario, cwd]) => {
	const { spawn } = ns;
	project = /** @type {string} */ (cwd);
	const prototype = Object.getPrototypeOf(spawn);
	switch (scenario) {
		case 'shape': {
			const program = spawn.program('./spawn/bin/tool');
			const narrowed = spawn.narrow(['tool']);
			return {
				namespace: describe(ns),
				root: describe(spawn),
				prototype: describe(prototype),
				methods: Object.fromEntries(
					['program', 'find', 'program_in', 'find_in', 'narrow'].map((name) => [
						name,
						{ length: prototype[name].length, name: prototype[name].name }
					])
				),
				program: describe(program),
				program_prototype: describe(Object.getPrototypeOf(program)),
				program_methods: Object.fromEntries(
					['run', 'out', 'out_bytes', 'start'].map((name) => {
						const method = Object.getPrototypeOf(program)[name];
						return [name, { length: method.length, name: method.name }];
					})
				),
				narrowed: describe(narrowed),
				narrowed_shares_the_prototype: Object.getPrototypeOf(narrowed) === prototype,
				same_handle_twice: spawn.program('./spawn/bin/tool') === program,
				then: 'then' in program,
				write: thrown(() => {
					/** @type {any} */ (program).name = 'x';
				}),
				write_prototype: thrown(() => {
					prototype.extra = 1;
				})
			};
		}
		case 'names':
			return ask_all(spawn, NAMES);
		case 'outside':
			return ask_all(spawn, OUTSIDE);
		case 'invalid': {
			const name_of = (/** @type {unknown} */ name) => outcome(() => spawn.find(name));
			const options_of = (/** @type {unknown} */ options) =>
				outcome(() => spawn.find('tool', options));
			return {
				names: [1, '', 'a\0b', '\ud800', null, undefined, ['tool']].map(name_of),
				name_before_options: outcome(() => spawn.find(1, 1)),
				options: [
					1,
					'all',
					null,
					[],
					() => {},
					{ pass_env: 'x' },
					{ pass_env: 'ALL' },
					{ pass_env: ['A*B'] },
					{ pass_env: ['A**'] },
					{ pass_env: ['*'] },
					{ pass_env: [''] },
					{ pass_env: ['A=B'] },
					{ pass_env: ['A\0'] },
					{ pass_env: [1] },
					{ pass_env: ['A', , 'B'] },
					{ omit_env: ['A'] },
					{ pass_env: ['A'], omit_env: [] },
					{ pass_env: 'all', omit_env: 'A' },
					{ pass_env: 'all', omit_env: ['A*'] },
					{ pass_env: 'all', omit_env: ['A=1'] },
					{ env: 1 },
					{ env: [] },
					{ env: null },
					{ env: () => {} },
					{ env: { A: 1 } },
					{ env: { '': 'x' } },
					{ env: { 'A=B': 'x' } },
					{ env: { A: 'a\0' } },
					{ env: { A: '\udc00' } },
					{ pass_env: 1, omit_env: 1, env: 1 },
					{ pass_env: ['A', 'B_*'], env: { A: '', B: 'b' } },
					{ pass_env: 'all', omit_env: ['A', 'B'] },
					{ __proto__: { pass_env: 1 } }
				].map(options_of),
				narrow: [1, 'tool', [1], [''], ['a\0'], { length: 0 }].map((names) =>
					outcome(() => spawn.narrow(/** @type {any} */ (names)))
				),
				program_in: [
					outcome(() => spawn.program_in(/** @type {any} */ ({}), 'tool')),
					outcome(() => spawn.find_in(/** @type {any} */ (null), 'tool')),
					outcome(() =>
						spawn.program_in(/** @type {any} */ ({ path: project }), /** @type {any} */ (1))
					)
				]
			};
		}
		case 'narrow': {
			const narrowed = spawn.narrow(['tool', './spawn/bin/tool', 'tool']);
			return {
				program: ask_all(narrowed, ['tool', './spawn/bin/tool', 'elf']),
				narrow: [['tool'], [], ['elf'], ['tool', 'elf']].map((names) =>
					outcome(() => narrowed.narrow(names))
				),
				narrowed_twice: outcome(() => narrowed.narrow(['tool']).find('tool')),
				emptied: outcome(() => narrowed.narrow([]).find('tool')),
				options: [
					{ env: {} },
					{ pass_env: 'all' },
					{ pass_env: ['GRO_A', 'GRO_P_*'] },
					{ pass_env: ['GRO_A'] },
					{ pass_env: 'all', env: { A: 'a' } }
				].map((options) => outcome(() => narrowed.find('tool', options))),
				program_in: outcome(() => narrowed.program_in(/** @type {any} */ ({}), 'tool'))
			};
		}
		case 'receiver': {
			const program = spawn.program('./spawn/bin/tool');
			const program_prototype = Object.getPrototypeOf(program);
			const borrowed = /** @type {Array<[string, unknown]>} */ ([
				['plain', {}],
				['null', null],
				['look-alike', Object.create(prototype)],
				['proxy', new Proxy(spawn, {})],
				['program', program]
			]);
			return {
				root: Object.fromEntries(
					borrowed.map(([label, receiver]) => [
						label,
						thrown(() => prototype.find.call(receiver, 'tool'))
					])
				),
				narrow: thrown(() => prototype.narrow.call({}, ['tool'])),
				program: Object.fromEntries(
					await Promise.all(
						['run', 'out', 'out_bytes', 'start'].map(async (name) => [
							name,
							// the class alone: Node's message is the engine's, of a private field
							(await settled(() => program_prototype[name].call(spawn, []))).rejected?.class ??
								'fulfilled'
						])
					)
				)
			};
		}
		case 'not_granted':
			return {
				program: ask_all(spawn, ['tool', 'elf', './spawn/bin/elf', 'sh']),
				narrow: outcome(() => spawn.narrow(['tool', 'elf']))
			};
		case 'forward':
			return Object.fromEntries(
				/** @type {Array<[string, unknown]>} */ ([
					['none', undefined],
					['named', { pass_env: ['GRO_A'] }],
					[
						'the base set',
						{ pass_env: ['PATH', 'HOME', 'TMPDIR', 'TERM', 'NO_COLOR', 'FORCE_COLOR'] }
					],
					['covered by a glob', { pass_env: ['GRO_P_X'] }],
					['a glob within a glob', { pass_env: ['GRO_P_*', 'GRO_P_Q*'] }],
					['env', { env: { GRO_A: 'set' } }],
					['ungranted', { pass_env: ['GRO_B'] }],
					['ungranted glob', { pass_env: ['GRO_*'] }],
					['all', { pass_env: 'all' }]
				]).map(([label, options]) => [label, outcome(() => spawn.program('tool', options))])
			);
		default:
			throw new Error(`no scenario ${scenario}`);
	}
};
