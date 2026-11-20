import { describe, thrown } from '../describe.js';

// The spawn host module's handles, over the programs under `spawn/`: `bin/` holds a `#!` script
// (`tool`), a link to it (`link`), a file with ELF's magic (`elf`), a `#!` naming no interpreter
// (`bare`), which pins as a script, one without execute permission (`noexec`), one that's
// neither a script nor a binary (`text`), and a `.cmd`;
// `dir/` holds only a file without execute permission; `nm/node_modules/.bin`, a link to
// `shadow/.bin`, holds a program a `PATH` walk skips by that spelling; and `rel/` holds one only
// a relative `PATH` entry names, which the walk skips too; `bin/emit` is a `#!` script whose
// first argument says what it does when it's started. Each case's `PATH` names these by `{cwd}`,
// the fixtures' directory, and each path a value or an error names is shown with the project's
// path as `{project}`, since the checkout's path is the machine's. The cases that start a program
// start `emit`, and `run`'s children, which share this process's stdout, print nothing.

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

/** A call's result as data: each own key in order, a `Uint8Array` as its bytes and whether
 * it holds its own buffer, exactly its length, and the record's shape. */
const show_result = (/** @type {any} */ result) => {
	if (result === null || typeof result !== 'object') return { not_a_record: typeof result };
	/** @type {Record<string, unknown>} */
	const fields = {};
	// the order too, which a comparison of `fields` as maps doesn't see
	const keys = Reflect.ownKeys(result).map(String);
	for (const key of Reflect.ownKeys(result)) {
		const value = result[key];
		fields[String(key)] =
			value instanceof Uint8Array
				? {
						bytes: Array.from(value),
						class: value.constructor.name,
						own_buffer: value.byteOffset === 0 && value.buffer.byteLength === value.length
					}
				: value;
	}
	return {
		fields,
		keys,
		frozen: Object.isFrozen(result),
		null_prototype: Object.getPrototypeOf(result) === null
	};
};

/** What a call settled to, as data: its result, or what it rejected with. */
const called = async (/** @type {() => Promise<unknown>} */ call) => {
	try {
		return { fulfilled: show_result(await call()) };
	} catch (error) {
		return { rejected: show_error(error) };
	}
};

/** What a call settled to, as `called` shows it, its `stdout` with the project's path as
 * `{project}`. */
const called_relative = async (/** @type {() => Promise<unknown>} */ call) => {
	const shown = /** @type {any} */ (await called(call));
	const fields = shown.fulfilled?.fields;
	if (fields) fields.stdout = relative(fields.stdout);
	return shown;
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

/** The names a case asks for beneath `spawn/bin` by `program_in` and `find_in`. */
const IN_NAMES = [
	'tool',
	'link',
	'elf',
	'bare',
	'noexec',
	'text',
	'run.cmd',
	'RUN.CMD. ',
	'missing',
	'missing/x',
	'tool/x',
	'sub/../tool',
	'./tool',
	'.',
	'tool/',
	'tool/.',
	'../bin/tool',
	'../../spawn/bin/tool',
	'/bin/sh'
];

/** The names a case asks for that only an unconfined run reaches: absolute, and climbing out. */
const OUTSIDE = ['/bin/sh', '../spawn/bin/tool', 'spawn/../../x'];

/**
 * @param {typeof import('../../../../../../src/lib/spawn.ts')} ns
 * @param {Array<string>} args
 */
export default async (ns, [scenario, cwd], _print, /** @type {any} */ load) => {
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
		case 'launch': {
			const emit = spawn.program('emit');
			const by_path = spawn.program('./spawn/bin/emit');
			const sh = spawn.program('sh');
			const with_env = spawn.program('emit', {
				pass_env: ['GRO_A', 'GRO_P_*'],
				env: { GRO_SET: 'set', PATH: '/usr/bin:rel:/bin' }
			});
			const big = new Uint8Array(200000).fill(98);
			return {
				run: {
					exited: await called(() => emit.run(['exit', '3'])),
					zero: await called(() => emit.run(['exit', '0'])),
					signaled: await called(() => emit.run(['signal', 'USR1'])),
					real_time: await called(() => emit.run(['signal', '35'])),
					by_signal: Object.fromEntries(
						await Promise.all(
							['USR2', 'ALRM', 'ABRT', 'TERM'].map(async (signal) => [
								signal,
								await called(() => emit.run(['signal', signal]))
							])
						)
					),
					timed_out: await called(() => emit.run(['sleep', '5'], { timeout_ms: 100 })),
					// its `SIGTERM` ignored, it's ended by the grace's `SIGKILL`
					timed_out_stubborn: await called(() => emit.run(['stubborn'], { timeout_ms: 100 })),
					// it exits on the `SIGTERM`, so the result has its code and no signal
					timed_out_exits: await called(() => emit.run(['trap'], { timeout_ms: 300 })),
					timed_out_real_time: await called(() =>
						emit.run(['real_time_on_term'], { timeout_ms: 300 })
					),
					extra_options: await called(() => emit.run(['exit', '0'], { max_bytes: -1, stdin: 1 }))
				},
				out: {
					streams: await called(() => emit.out(['streams'])),
					args: await called(() => emit.out(['args', 'a b', '', 'é', '--x'])),
					argv0_script_bare: await called_relative(() => emit.out(['argv0'])),
					argv0_script_path: await called_relative(() => by_path.out(['argv0'])),
					argv0_native: await called(() => sh.out(['-c', 'printf %s "$0"'])),
					exited: await called(() => emit.out(['exit', '5'])),
					killed: await called(() => emit.out(['signal', 'TERM'])),
					text: await called(() => emit.out(['bytes'])),
					pwd: await called_relative(() => emit.out(['pwd'])),
					vars: await called_relative(() =>
						emit.out(['vars', 'PATH', 'HOME', 'LC_ALL', 'GRO_A', 'GRO_B', 'GRO_P_X', 'GRO_SECRET'])
					),
					vars_given: await called(() =>
						with_env.out(['vars', 'PATH', 'LC_ALL', 'GRO_A', 'GRO_B', 'GRO_P_X', 'GRO_SET'])
					),
					stdin_ignored: await called(() => emit.out(['cat'])),
					stdin_data: await called(() =>
						emit.out(['cat'], { stdin: new TextEncoder().encode('fed') })
					),
					stdin_empty: await called(() => emit.out(['cat'], { stdin: new Uint8Array(0) })),
					stdin_large: await called(async () => {
						const result = /** @type {any} */ (await emit.out_bytes(['cat'], { stdin: big }));
						return { ...result, stdout: result.stdout.length };
					}),
					stdin_unread: await called(() => emit.out(['exit', '0'], { stdin: big })),
					capped: await called(() => emit.out(['hold'], { max_bytes: 3 })),
					capped_none: await called(() => emit.out(['hold'], { max_bytes: 0 })),
					at_the_cap: await called(() => emit.out(['streams'], { max_bytes: 3 })),
					timed_out: await called(() => emit.out(['sleep', '5'], { timeout_ms: 100 })),
					timed_out_exits: await called(() => emit.out(['trap'], { timeout_ms: 300 })),
					lingering: await called(() => emit.out(['linger']))
				},
				out_bytes: {
					streams: await called(() => emit.out_bytes(['streams'])),
					bytes: await called(() => emit.out_bytes(['bytes'])),
					capped: await called(() => emit.out_bytes(['hold'], { max_bytes: 4 })),
					empty: await called(() => emit.out_bytes(['exit', '0']))
				}
			};
		}
		case 'call_invalid': {
			const emit = spawn.program('emit');
			const args_of = (/** @type {unknown} */ args) =>
				called(() => emit.out(/** @type {any} */ (args)));
			const options_of = (/** @type {unknown} */ options) =>
				called(() => emit.out(['exit', '0'], /** @type {any} */ (options)));
			return {
				args: await Promise.all(
					[undefined, 'x', {}, [1], ['a\0'], ['\ud800'], ['a', , 'b'], [['x']]].map(args_of)
				),
				options: await Promise.all(
					[
						1,
						null,
						'x',
						[],
						() => {},
						{ cwd: {} },
						{ cwd: null },
						{ timeout_ms: 0 },
						{ timeout_ms: -1 },
						{ timeout_ms: 1.5 },
						{ timeout_ms: 2147483648 },
						{ timeout_ms: '1' },
						{ timeout_ms: Number.NaN },
						{ max_bytes: -1 },
						{ max_bytes: 1.5 },
						{ max_bytes: 2 ** 53 },
						{ max_bytes: '1' },
						{ stdin: 'pipe' },
						{ stdin: 1 },
						{ stdin: null },
						{ stdin: [1] },
						{ stdin: new Uint16Array(1) },
						{ timeout_ms: 0, max_bytes: -1, stdin: 1 },
						{ max_bytes: -1, stdin: 1 },
						{ __proto__: { timeout_ms: 0 } }
					].map(options_of)
				),
				args_before_options: await called(() => emit.out(/** @type {any} */ (1), 1)),
				run_options: await Promise.all(
					[1, { cwd: {} }, { timeout_ms: 0 }].map((options) =>
						called(() => emit.run(['exit', '0'], /** @type {any} */ (options)))
					)
				)
			};
		}
		case 'in': {
			const { fs } = /** @type {any} */ (await load('fs'));
			const bin = fs.project.dir('spawn/bin');
			const ask_in = (/** @type {any} */ dir, /** @type {string} */ name) => ({
				program_in: outcome(() => spawn.program_in(dir, name)),
				find_in: outcome(() => spawn.find_in(dir, name))
			});
			return {
				names: Object.fromEntries(IN_NAMES.map((name) => [name, ask_in(bin, name)])),
				nested: ask_in(fs.project.dir('spawn'), 'bin/tool'),
				read_only: ask_in(bin.readonly(), 'tool'),
				argv0: await called_relative(() => spawn.program_in(bin, 'emit').out(['argv0'])),
				args: await called(() => spawn.program_in(bin, 'emit').out(['args', 'a'])),
				pwd: await called_relative(() => spawn.program_in(bin, 'emit').out(['pwd'], { cwd: bin })),
				forged: [{}, null, { path: bin.path }, Object.create(Object.getPrototypeOf(bin))].map(
					(dir) => outcome(() => spawn.find_in(/** @type {any} */ (dir), 'tool'))
				),
				narrowed: outcome(() => spawn.narrow([]).find_in(bin, 'tool')),
				forwarded: outcome(() => spawn.find_in(bin, 'tool', { pass_env: ['GRO_A'] }))
			};
		}
		case 'in_outside': {
			const { fs } = /** @type {any} */ (await load('fs'));
			return {
				project: outcome(() => spawn.find_in(fs.project, './spawn/bin/tool')),
				above: outcome(() => spawn.find_in(fs.project.dir('spawn'), 'bin/tool'))
			};
		}
		case 'in_scratch': {
			const { fs } = /** @type {any} */ (await load('fs'));
			const scratch = await fs.open_dir(`${project}/spawn_scratch`);
			await scratch.write('built', '#!/bin/sh\nprintf built\n');
			await scratch.set_mode('built', 0o755);
			const built = spawn.program_in(scratch, 'built');
			return {
				built: show_program(built),
				ran: await called(() => built.out([])),
				links: Object.fromEntries(
					['inside', 'absolute', 'escape', 'dangling', 'loop'].map((name) => [
						name,
						outcome(() => spawn.find_in(scratch, name))
					])
				)
			};
		}
		case 'start': {
			const emit = spawn.program('emit');
			/** Every chunk of `stream` read, as text, and whether each was a plain `Uint8Array` of
			 * its own buffer in a null-prototype `{done, value}` record. */
			const read_all = async (/** @type {any} */ stream) =>
				read_iterator(stream[Symbol.asyncIterator]());
			/** As `read_all`, from the iterator a stream made. */
			const read_iterator = async (/** @type {any} */ iterator) => {
				/** @type {Array<number>} */
				const bytes = [];
				let plain = true;
				for (;;) {
					const result = await iterator.next();
					plain &&=
						Object.getPrototypeOf(result) === null &&
						Reflect.ownKeys(result).join() === 'done,value';
					if (result.done) break;
					const { value } = result;
					plain &&=
						value.constructor === Uint8Array &&
						value.byteOffset === 0 &&
						value.buffer.byteLength === value.length;
					bytes.push(...value);
				}
				return { text: new TextDecoder().decode(new Uint8Array(bytes)), plain };
			};
			const piped = /** @type {any} */ (await emit.start(['streams'], { stdio: 'pipe' }));
			const stderr_iterator = piped.stderr[Symbol.asyncIterator]();
			const piped_shape = {
				iterator: describe(stderr_iterator),
				iterator_prototype: describe(Object.getPrototypeOf(stderr_iterator)),
				child: describe(piped),
				prototype: describe(Object.getPrototypeOf(piped)),
				stdout: describe(piped.stdout),
				stdout_prototype: describe(Object.getPrototypeOf(piped.stdout)),
				stdin: describe(piped.stdin),
				stdin_prototype: describe(Object.getPrototypeOf(piped.stdin)),
				then: 'then' in piped,
				pid: typeof piped.pid,
				closed_frozen: Object.isFrozen(piped.closed),
				// each method's `length` and `name`, which `describe` leaves out
				methods: Object.fromEntries(
					/** @type {Array<[string, any, Array<string | symbol>]>} */ ([
						['child', Object.getPrototypeOf(piped), ['kill', Symbol.asyncDispose]],
						['stdin', Object.getPrototypeOf(piped.stdin), ['write', 'close']],
						['stdout', Object.getPrototypeOf(piped.stdout), [Symbol.asyncIterator]],
						['iterator', Object.getPrototypeOf(stderr_iterator), ['next', 'return', 'throw']]
					]).map(([of, prototype, keys]) => [
						of,
						keys.map((key) => ({ length: prototype[key].length, name: prototype[key].name }))
					])
				)
			};
			const piped_read = {
				stdout: await read_all(piped.stdout),
				stderr: await read_iterator(stderr_iterator),
				second_iteration: thrown(() => piped.stdout[Symbol.asyncIterator]()),
				closed: show_result(await piped.closed),
				kill_after_the_end: piped.kill(),
				closed_again: (await piped.closed) === (await piped.closed)
			};
			const inherited = /** @type {any} */ (await emit.start(['exit', '3']));
			const mixed = /** @type {any} */ (
				await emit.start(['streams'], { stdio: ['ignore', 'pipe', 'ignore'] })
			);
			const fed = /** @type {any} */ (
				await emit.start(['echo_err'], { stdio: ['pipe', 'pipe', 'pipe'] })
			);
			// written and closed at once, never awaited between
			const writes = [
				fed.stdin.write('a'),
				fed.stdin.write(new TextEncoder().encode('b')),
				fed.stdin.write(''),
				fed.stdin.close(),
				fed.stdin.write('late'),
				fed.stdin.close()
			];
			const fed_read = await Promise.all([read_all(fed.stdout), read_all(fed.stderr)]);
			// more than a pipe holds, written and closed at once, then written again, the output read
			// as it comes: the late write goes nowhere, and takes nothing of what came before
			const big_fed = /** @type {any} */ (
				await emit.start(['cat'], { stdio: ['pipe', 'pipe', 'ignore'] })
			);
			const big_writes = Promise.all([
				big_fed.stdin.write(new Uint8Array(200000).fill(98)),
				big_fed.stdin.close(),
				big_fed.stdin.write('late')
			]);
			const big_read = await read_all(big_fed.stdout);
			const big = {
				writes: await big_writes,
				length: big_read.text.length,
				closed: show_result(await big_fed.closed)
			};
			const sleeper = /** @type {any} */ (await emit.start(['sleep', '5'], { stdio: 'ignore' }));
			const disposed = /** @type {any} */ (await emit.start(['sleep', '5'], { stdio: 'ignore' }));
			const stubborn = /** @type {any} */ (await emit.start(['stubborn'], { stdio: 'ignore' }));
			const timed = /** @type {any} */ (
				await emit.start(['sleep', '5'], { stdio: 'ignore', timeout_ms: 100 })
			);
			const many = /** @type {any} */ (
				await emit.start(['big', '3000000'], { stdio: ['ignore', 'pipe', 'ignore'] })
			);
			// left behind the pause, then read whole
			await new Promise((resolve) => setTimeout(resolve, 300));
			const many_read = await read_all(many.stdout);
			const left = /** @type {any} */ (
				await emit.start(['big', '3000000'], { stdio: ['ignore', 'pipe', 'ignore'] })
			);
			let first_length = 0;
			for await (const chunk of left.stdout) {
				first_length = chunk.length;
				break;
			}
			// thrown into after its first chunk: the rest dropped, the error rejected, then done
			const thrown_into = /** @type {any} */ (
				await emit.start(['big', '3000000'], { stdio: ['ignore', 'pipe', 'ignore'] })
			);
			const thrown_iterator = thrown_into.stdout[Symbol.asyncIterator]();
			const thrown_first = await thrown_iterator.next();
			const thrown_rejection = await thrown_iterator.throw(new RangeError('stop')).then(
				() => 'resolved',
				(/** @type {any} */ error) => `${error.constructor.name}: ${error.message}`
			);
			const thrown_after = await thrown_iterator.next();
			return {
				piped: piped_shape,
				piped_read,
				inherited: {
					stdout: inherited.stdout,
					stderr: inherited.stderr,
					stdin: inherited.stdin,
					closed: show_result(await inherited.closed)
				},
				mixed: {
					stdin: mixed.stdin,
					stdout: await read_all(mixed.stdout),
					stderr: mixed.stderr,
					closed: show_result(await mixed.closed)
				},
				fed: {
					writes: await Promise.all(writes),
					stdout: fed_read[0],
					stderr: fed_read[1],
					invalid: await Promise.all(
						[1, '\ud800', null, [1]].map((data) => settled(() => fed.stdin.write(data)))
					),
					closed: show_result(await fed.closed)
				},
				big_fed: big,
				kill: {
					names: ['SIGNOPE', 9, 'sigterm', null].map((signal) =>
						thrown(() => sleeper.kill(signal))
					),
					sent: sleeper.kill(),
					closed: show_result(await sleeper.closed),
					after_the_end: sleeper.kill('SIGKILL')
				},
				disposed: {
					disposal: await disposed[Symbol.asyncDispose](),
					closed: show_result(await disposed.closed),
					again: await disposed[Symbol.asyncDispose]()
				},
				// its `SIGTERM` ignored, it's killed by the grace's `SIGKILL`
				disposed_stubborn: show_result(
					await stubborn[Symbol.asyncDispose]().then(() => stubborn.closed)
				),
				timed_out: show_result(await timed.closed),
				behind_the_pause: { length: many_read.text.length, closed: show_result(await many.closed) },
				left_early: {
					first: first_length > 0,
					after: await read_all({
						[Symbol.asyncIterator]: () => left.stdout[Symbol.asyncIterator]()
					}).catch((error) => error.constructor.name),
					closed: show_result(await left.closed)
				},
				thrown_into: {
					first: thrown_first.done === false && thrown_first.value.length > 0,
					rejected: thrown_rejection,
					after: [thrown_after.done, thrown_after.value === undefined],
					again: await read_iterator(thrown_iterator),
					closed: show_result(await thrown_into.closed)
				},
				invalid: await Promise.all(
					[{ stdio: 'x' }, { stdio: ['pipe'] }, { stdio: ['pipe', 'pipe', 'x'] }, { stdio: 1 }].map(
						(options) => called(() => emit.start(['exit', '0'], /** @type {any} */ (options)))
					)
				)
			};
		}
		default:
			throw new Error(`no scenario ${scenario}`);
	}
};
