// Imports every builtin module (`modules.js`, which the test writes from the builtin table),
// exercises every export (`exercise.js`: each function called with no arguments, with the
// arguments `EXERCISES` gives for it, with hostile arguments, and with a hostile receiver, a
// class constructed, a getter read, a setter given a value, and the same for what each
// returns, throws, or holds), and reports what each namespace, export, and call's outcome is,
// as it goes. Loaded by `import()` from an entry that armed the traps first, the builtins'
// backing scripts ran armed: this module's imports are their first.
//
// A call that ends the process, as `exit`'s do, ends the run there, short of its end: the
// test runs the entry again with that call in `skips.js`, which it writes, until a run gets
// to its end, and holds each call that ended one to the code it exited with.

import { MODULES } from './modules.js';
import { ENDED } from './skips.js';
import { create_exerciser } from './exercise.js';
import { count_replaced, disarm, report, restore_realm, show } from './realm.js';

disarm();

/** A row's namespace, by its specifier. */
const namespace_of = (/** @type {string} */ specifier) =>
	/** @type {any} */ (MODULES.find(([row]) => row === specifier))[2];

/** The env error row's namespace, whose class makes the errors its guard is given. */
const env_error = namespace_of('@fuzdev/gro/env_error.ts');

/** The fs error row's namespace, the same for its errors. */
const fs_error = namespace_of('@fuzdev/gro/fs_error.ts');

/** The spawn error row's namespace, the same for its errors. */
const spawn_error = namespace_of('@fuzdev/gro/spawn_error.ts');

/** The project's path, which the run reads and writes beneath. */
const project = namespace_of('@fuzdev/gro/fs.ts').fs.project.path;

/** A path no handle reaches, refused before any grant is checked or any file read. */
const ABOVE = '..';

/**
 * A handle on `w`, the directory the run writes in its working directory, made before the
 * realm is armed: the writes built on fgro are exercised through it as well as `fs.project`,
 * which doesn't write.
 */
const written = await namespace_of('@fuzdev/gro/fs.ts').fs.open_dir(
	`${namespace_of('@fuzdev/gro/cwd.ts').cwd()}/w`
);

/**
 * A temporary directory the run's grant makes, made before the realm is armed, whose disposal is
 * exercised below.
 */
const temp = await namespace_of('@fuzdev/gro/fs.ts').fs.temp_dir('exercised-');

/**
 * A child of `tool`, which exits at once, started with its streams piped before the realm is
 * armed: its methods, its stdin's, and its streams' and their iterators' are exercised through
 * it, as a call's result is past the walk's reach.
 */
const started = await namespace_of('@fuzdev/gro/spawn.ts').spawn
	.program('tool')
	.start([], { stdio: 'pipe' });

/** The iterator of the started child's stderr, taken before the realm is armed, past the
 * walk's reach, so its methods are exercised. */
const started_iterator = started.stderr[Symbol.asyncIterator]();

/**
 * The arguments each function or class a builtin exports or reaches is exercised with, by its
 * path as the reports name it (`<specifier> <export>`, then `.member` or `()` for each step):
 * lists that reach what a call with no argument or with hostile ones doesn't, and none for a
 * function that takes nothing. A function reached that has no entry here fails the test,
 * whatever it declares, and so does an entry nothing was reached at.
 * @type {Map<string, Array<Array<unknown>>>}
 */
const EXERCISES = new Map([
	// takes no argument and no receiver
	['@fuzdev/gro/cwd.ts cwd', []],
	// a kind and a target, and with them the options of its cause
	[
		'@fuzdev/gro/env_error.ts EnvError',
		[
			['not_granted', 'HOME'],
			['not_granted', 'HOME', { __proto__: null, cause: 1 }]
		]
	],
	// an error of the class, of its kind and of another, and one that isn't
	[
		'@fuzdev/gro/env_error.ts is_env_error',
		[
			[new env_error.EnvError('not_granted', 'HOME'), 'not_granted'],
			[new env_error.EnvError('not_granted', 'HOME'), 'other'],
			[{ __proto__: null }]
		]
	],
	// a name the run grants that's set, and one that's unset; with a hostile array in its place,
	// `narrow` is given the one name `hostile`, which the run grants too, as each handle here has
	['@fuzdev/gro/env.ts env.get', [['GRO_PATCHED_SET'], ['GRO_PATCHED_UNSET']]],
	['@fuzdev/gro/env.ts env.get_all', []],
	['@fuzdev/gro/env.ts env.narrow', [[['GRO_PATCHED_SET', 'GRO_PATCHED_UNSET', 'hostile']]]],
	// the same of the handle that call narrowed to
	['@fuzdev/gro/env.ts env.narrow(arguments 1).get', [['GRO_PATCHED_SET'], ['hostile']]],
	['@fuzdev/gro/env.ts env.narrow(arguments 1).get_all', []],
	['@fuzdev/gro/env.ts env.narrow(arguments 1).narrow', [[['hostile', 'GRO_PATCHED_SET']]]],
	// a code, which ends the process as a call with none does
	['@fuzdev/gro/exit.ts exit', [[7]]],
	// a code, the one a run that ends on its own exits with
	['@fuzdev/gro/exit.ts set_exit_code', [[0]]],
	// bytes, a key of 32 bytes, and a context, and the same of each hasher those calls made
	['@fuzdev/gro/blake3.ts hash', [[new Uint8Array([1, 2, 3])]]],
	['@fuzdev/gro/blake3.ts create_hasher', []],
	['@fuzdev/gro/blake3.ts create_keyed_hasher', [[new Uint8Array(32)]]],
	['@fuzdev/gro/blake3.ts create_derive_key_hasher', [['context']]],
	['@fuzdev/gro/blake3.ts create_hasher().update', [[new Uint8Array([1])]]],
	['@fuzdev/gro/blake3.ts create_hasher().finalize', []],
	['@fuzdev/gro/blake3.ts create_keyed_hasher(arguments 1).update', [[new Uint8Array([1])]]],
	['@fuzdev/gro/blake3.ts create_keyed_hasher(arguments 1).finalize', []],
	['@fuzdev/gro/blake3.ts create_derive_key_hasher(arguments 1).update', [[new Uint8Array([1])]]],
	['@fuzdev/gro/blake3.ts create_derive_key_hasher(arguments 1).finalize', []],
	// a source of each language, and TypeScript's with its options, so a hostile one in their
	// place is read; and the same of a stack the factory made: layers pushed and popped, and a
	// path matched
	[
		'@fuzdev/gro/tsv.ts format_typescript',
		[['let x=1'], ['let x=1', { __proto__: null, sourceType: 'module' }]]
	],
	['@fuzdev/gro/tsv.ts format_svelte', [['<p>{ x }</p>']]],
	['@fuzdev/gro/tsv.ts format_css', [['a{color:red}']]],
	[
		'@fuzdev/gro/tsv.ts parse_imports_typescript',
		[["import a from 'a';", { __proto__: null, sourceType: 'module' }]]
	],
	['@fuzdev/gro/tsv.ts parse_imports_svelte', [["<script>import a from 'a';</script>"]]],
	['@fuzdev/gro/tsv.ts create_ignore_stack', []],
	['@fuzdev/gro/tsv.ts create_ignore_stack().push_gitignore', [['', 'dist/']]],
	['@fuzdev/gro/tsv.ts create_ignore_stack().pop_gitignore', []],
	['@fuzdev/gro/tsv.ts create_ignore_stack().push_formatignore', [['', '!dist/x']]],
	['@fuzdev/gro/tsv.ts create_ignore_stack().push_prettierignore', [['', '*.md']]],
	['@fuzdev/gro/tsv.ts create_ignore_stack().pop_override', []],
	['@fuzdev/gro/tsv.ts create_ignore_stack().is_ignored', [['dist', true]]],
	// a kind and a path, and with them the options of its cause
	[
		'@fuzdev/gro/fs_error.ts FsError',
		[
			['not_found', '/x'],
			['not_found', '/x', { __proto__: null, cause: 1 }]
		]
	],
	// an error of the class, of its kind and of another, and one that isn't
	[
		'@fuzdev/gro/fs_error.ts is_fs_error',
		[
			[new fs_error.FsError('not_found', '/x'), 'not_found'],
			[new fs_error.FsError('not_found', '/x'), 'other'],
			[{ __proto__: null }]
		]
	],
	['@fuzdev/gro/fs_error.ts is_own_fs_error', [[new fs_error.FsError('not_found', '/x')]]],
	// the project's directory and a file in it, which the run reads
	['@fuzdev/gro/fs.ts fs.open_dir', [[project]]],
	['@fuzdev/gro/fs.ts fs.real_path', [[`${project}/package.json`]]],
	// a prefix, so a hostile one in its place is read
	['@fuzdev/gro/fs.ts fs.temp_dir', [['made-']]],
	// the temporary directory above, removed, then removed again
	['@fuzdev/gro/fs.ts temp.Symbol(Symbol.asyncDispose)', [[], []]],
	// a file, a missing path and a final link kept, and a directory, beneath the project
	['@fuzdev/gro/fs.ts fs.project.stat', [['package.json'], ['missing', { follow: false }]]],
	['@fuzdev/gro/fs.ts fs.project.stat_sync', [['package.json'], ['missing', { follow: false }]]],
	['@fuzdev/gro/fs.ts fs.project.exists', [['package.json']]],
	['@fuzdev/gro/fs.ts fs.project.exists_sync', [['package.json']]],
	['@fuzdev/gro/fs.ts fs.project.dir', [['.']]],
	['@fuzdev/gro/fs.ts fs.project.readonly', []],
	// a file beneath the project, and a path with nothing at it; and a listing of the project
	['@fuzdev/gro/fs.ts fs.project.read', [['package.json']]],
	['@fuzdev/gro/fs.ts fs.project.read_text', [['package.json']]],
	['@fuzdev/gro/fs.ts fs.project.read_if_exists', [['package.json'], ['missing']]],
	['@fuzdev/gro/fs.ts fs.project.read_text_if_exists', [['package.json'], ['missing']]],
	['@fuzdev/gro/fs.ts fs.project.read_text_sync', [['package.json']]],
	['@fuzdev/gro/fs.ts fs.project.read_link', [['package.json']]],
	['@fuzdev/gro/fs.ts fs.project.list', [['.']]],
	// the listing below, its first `next()` reading the project, then a value it settles to
	// on `return`, and an error it rejects with on `throw`
	['@fuzdev/gro/fs.ts listing.next', []],
	['@fuzdev/gro/fs.ts listing.return', [[1]]],
	['@fuzdev/gro/fs.ts listing.throw', [[1]]],
	['@fuzdev/gro/fs.ts listing.Symbol(Symbol.asyncIterator)', []],
	// each write given a path above the handle, refused before the grant it lacks, which
	// would fail the run
	['@fuzdev/gro/fs.ts fs.project.write', [[ABOVE, 'data']]],
	['@fuzdev/gro/fs.ts fs.project.mkdir', [[ABOVE]]],
	['@fuzdev/gro/fs.ts fs.project.remove', [[ABOVE]]],
	['@fuzdev/gro/fs.ts fs.project.rename', [[ABOVE, 'x']]],
	['@fuzdev/gro/fs.ts fs.project.copy', [[ABOVE, 'x']]],
	['@fuzdev/gro/fs.ts fs.project.set_times', [[ABOVE, { mtime_ms: 0 }]]],
	['@fuzdev/gro/fs.ts fs.project.set_mode', [[ABOVE, 0o644]]],
	['@fuzdev/gro/fs.ts fs.project.symlink', [[ABOVE, 'x']]],
	// each write built on fgro through `written`, a handle on a directory the run writes, with
	// data and options, so a hostile one in their place is read: a file written, made new, a
	// directory made, a file renamed, a missing file and a directory removed, a file's times and
	// mode set, a link made, and a file copied and a tree copied into itself, refused
	[
		'@fuzdev/gro/fs.ts written.write',
		[
			['written.txt', 'data'],
			['made.txt', new Uint8Array([1]), { create_new: true, mode: 0o600 }]
		]
	],
	['@fuzdev/gro/fs.ts written.mkdir', [['made', { recursive: true, exists_ok: true }]]],
	[
		'@fuzdev/gro/fs.ts written.remove',
		[
			['missing', { missing_ok: true }],
			['made', { recursive: true }]
		]
	],
	['@fuzdev/gro/fs.ts written.rename', [['written.txt', 'renamed.txt']]],
	[
		'@fuzdev/gro/fs.ts written.set_times',
		[['made.txt', { mtime_ms: 0, atime_ms: 1 }, { follow: false }]]
	],
	['@fuzdev/gro/fs.ts written.set_mode', [['made.txt', 0o644]]],
	['@fuzdev/gro/fs.ts written.symlink', [['link', 'made.txt']]],
	[
		'@fuzdev/gro/fs.ts written.copy',
		[
			['made.txt', 'copied.txt'],
			['.', 'tree', { recursive: true }]
		]
	],
	// a kind and a program, and with them the options of its cause
	[
		'@fuzdev/gro/spawn_error.ts SpawnError',
		[
			['not_found', 'git'],
			['not_found', 'git', { __proto__: null, cause: 1 }]
		]
	],
	// an error of the class, of its kind and of another, and one that isn't
	[
		'@fuzdev/gro/spawn_error.ts is_spawn_error',
		[
			[new spawn_error.SpawnError('not_found', 'git'), 'not_found'],
			[new spawn_error.SpawnError('not_found', 'git'), 'other'],
			[{ __proto__: null }]
		]
	],
	// the program the run grants, with options, so a hostile one in their place is read; its
	// handle's launches each start `tool`, which exits at once, given arguments and options, so a
	// hostile one in either place is read; `start` given arguments, its streams inherited
	[
		'@fuzdev/gro/spawn.ts spawn.program',
		[['tool', { __proto__: null, pass_env: [], env: { __proto__: null, A: 'a' } }]]
	],
	['@fuzdev/gro/spawn.ts spawn.find', [['tool', { __proto__: null, pass_env: [] }]]],
	// with a hostile array in its place, `narrow` is given the one name `hostile`, which the run
	// grants and each root here holds, as for `env`
	['@fuzdev/gro/spawn.ts spawn.narrow', [[['tool', 'hostile']]]],
	[
		'@fuzdev/gro/spawn.ts spawn.program(arguments 1).run',
		[[['a'], { __proto__: null, timeout_ms: 10000 }]]
	],
	[
		'@fuzdev/gro/spawn.ts spawn.program(arguments 1).out',
		[[['a'], { __proto__: null, timeout_ms: 10000, max_bytes: 100, stdin: new Uint8Array(1) }]]
	],
	[
		'@fuzdev/gro/spawn.ts spawn.program(arguments 1).out_bytes',
		[[['a'], { __proto__: null, timeout_ms: 10000, max_bytes: 100, stdin: new Uint8Array(1) }]]
	],
	['@fuzdev/gro/spawn.ts spawn.program(arguments 1).start', [[[]]]],
	[
		'@fuzdev/gro/spawn.ts spawn.find(arguments 1).run',
		[[['a'], { __proto__: null, timeout_ms: 10000 }]]
	],
	[
		'@fuzdev/gro/spawn.ts spawn.find(arguments 1).out',
		[[['a'], { __proto__: null, timeout_ms: 10000, max_bytes: 100, stdin: new Uint8Array(1) }]]
	],
	[
		'@fuzdev/gro/spawn.ts spawn.find(arguments 1).out_bytes',
		[[['a'], { __proto__: null, timeout_ms: 10000, max_bytes: 100, stdin: new Uint8Array(1) }]]
	],
	['@fuzdev/gro/spawn.ts spawn.find(arguments 1).start', [[[]]]],
	['@fuzdev/gro/spawn.ts spawn.narrow(arguments 1).program', [['tool']]],
	['@fuzdev/gro/spawn.ts spawn.narrow(arguments 1).find', [['tool']]],
	['@fuzdev/gro/spawn.ts spawn.narrow(arguments 1).narrow', [[['tool']]]],
	// a directory and an empty name, refused before the exec roots it would need, which would
	// fail the run
	['@fuzdev/gro/spawn.ts spawn.program_in', [[written, '']]],
	['@fuzdev/gro/spawn.ts spawn.find_in', [[written, '']]],
	['@fuzdev/gro/spawn.ts spawn.narrow(arguments 1).program_in', [[written, '']]],
	['@fuzdev/gro/spawn.ts spawn.narrow(arguments 1).find_in', [[written, '']]],
	// the started child's: its streams' iterators, the first call taking each, the rest refused;
	// each iterator's `next()`, with its end, and its `return()` and `throw()`; its stdin's
	// writes, text and bytes, and its close; a signal sent, past the child's end or not; and its
	// disposal, twice
	['@fuzdev/gro/spawn.ts started.stdout.Symbol(Symbol.asyncIterator)', []],
	['@fuzdev/gro/spawn.ts started.stderr.Symbol(Symbol.asyncIterator)', []],
	['@fuzdev/gro/spawn.ts started.stdin.write', [['a'], [new Uint8Array(1)]]],
	['@fuzdev/gro/spawn.ts started.stdin.close', []],
	['@fuzdev/gro/spawn.ts started.kill', [['SIGTERM']]],
	['@fuzdev/gro/spawn.ts started.Symbol(Symbol.asyncDispose)', [[], []]],
	['@fuzdev/gro/spawn.ts started_iterator.next', []],
	['@fuzdev/gro/spawn.ts started_iterator.return', []],
	['@fuzdev/gro/spawn.ts started_iterator.throw', [[1]]]
]);

/** How far into an export's properties and its calls' results the functions are looked for. */
const DEPTH = 4;

const exerciser = create_exerciser(EXERCISES, DEPTH, ENDED);

for (const [specifier, exports, namespace] of MODULES) {
	// the namespace's own shape, each export shown below
	report('line', `${specifier}: ${show(namespace, 1)}`);
	for (const [name, per_process] of exports) {
		const value = namespace[name];
		// a value that is the process's own is shown by its type
		report('line', `${specifier} ${name}: ${per_process ? typeof value : show(value)}`);
		exerciser.exercise(`${specifier} ${name}`, value, undefined, DEPTH);
	}
}

// a listing's methods, past the depth the walk reaches through `fs.project.list()`: the
// iterator `list` returns, made before the realm is armed
exerciser.exercise(
	'@fuzdev/gro/fs.ts listing',
	namespace_of('@fuzdev/gro/fs.ts').fs.project.list(),
	undefined,
	DEPTH
);

// the writes built on fgro, through a handle that writes, each reached as it would be through
// `fs.project`, and nothing past them
const written_prototype = Object.getPrototypeOf(written);
for (const name of [
	'write',
	'mkdir',
	'remove',
	'rename',
	'copy',
	'set_times',
	'set_mode',
	'symlink'
]) {
	exerciser.exercise(`@fuzdev/gro/fs.ts written.${name}`, written_prototype[name], written, 1);
}

// a started child's methods and its streams', reached through it
exerciser.exercise('@fuzdev/gro/spawn.ts started', started, undefined, DEPTH);

exerciser.exercise('@fuzdev/gro/spawn.ts started_iterator', started_iterator, undefined, DEPTH);

// a temporary directory's disposal, reached through its handle
exerciser.exercise(
	'@fuzdev/gro/fs.ts temp.Symbol(Symbol.asyncDispose)',
	Object.getPrototypeOf(temp)[Symbol.asyncDispose],
	temp,
	1
);

const replaced = count_replaced();
restore_realm();
await exerciser.settle();
report('done', { replaced, unused: exerciser.unused() });
