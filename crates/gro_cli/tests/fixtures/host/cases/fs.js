import { describe, thrown } from '../describe.js';

// The fs host module over the tree under `fs/`, a directory of the project: files (an empty
// one, a large one, one whose bytes aren't all UTF-8), a subdirectory, a directory of names
// whose UTF-8 order isn't their UTF-16 order, and symlinks that stay inside, climb out, are
// absolute, and dangle; and a unix socket at `fs/socket` for the case that names it, which
// the case's runner makes, since git holds none. Each path a value or an error names is shown
// with the project's path as `{project}`, since the checkout's path is the machine's.

/** @type {string} */
let project = '';

/** `text` with the project's path as `{project}`. */
const relative = (/** @type {string} */ text) => text.split(project).join('{project}');

/** The last name of `path`, an absolute path. */
const name_of = (/** @type {string} */ path) => path.slice(path.lastIndexOf('/') + 1);

/** What an op's error is to a program, as data. */
const show_error = (/** @type {any} */ error) => {
	if (!(error instanceof Error)) return { not_an_error: typeof error };
	const { constructor, message, kind, path } = error;
	return {
		class: constructor?.name,
		message: relative(message),
		kind,
		path: typeof path === 'string' ? relative(path) : path
	};
};

/** A sum of `units`, each a byte or a UTF-16 code unit, times its place, to tell contents apart. */
const sum_of = (/** @type {ArrayLike<number>} */ units) => {
	let sum = 0;
	for (let i = 0; i < units.length; i++) sum = (sum + (i + 1) * units[i]) % 1_000_000_007;
	return sum;
};

/** The UTF-16 code units of `text`. */
const units_of = (/** @type {string} */ text) => Array.from(text, (_, i) => text.charCodeAt(i));

/** What an op returned or settled to, as data. */
const show_value = (/** @type {any} */ value) => {
	if (typeof value === 'string') {
		// the project's path swapped out first, so how a value shows doesn't depend on its length
		const text = relative(value);
		if (text.length <= 100) return text;
		return { text: text.length, sum: sum_of(units_of(text)), start: text.slice(0, 11) };
	}
	if (value === null || typeof value !== 'object') return value;
	if (value instanceof Uint8Array) {
		return {
			bytes: value.length,
			plain: Object.getPrototypeOf(value) === Uint8Array.prototype,
			sum: sum_of(value),
			start: Array.from(value.subarray(0, 4))
		};
	}
	if ('done' in value) return { done: value.done, value: show_value(value.value) };
	if (typeof value.name === 'string') return { name: value.name, kind: value.kind };
	if (typeof value.next === 'function') return 'a listing';
	if (typeof value.path === 'string') {
		return { handle: relative(value.path), writes: typeof value.write === 'function' };
	}
	const { kind, size, mtime_ms, mode } = value;
	return {
		kind,
		// a directory's size is its filesystem's
		size: kind === 'file' ? size : typeof size,
		mtime_ms: typeof mtime_ms,
		mode: typeof mode
	};
};

/**
 * How `call` ended: it threw, returned, or returned a promise that settled, each shown, so a
 * refusal that throws at the call is told from one that rejects.
 */
const outcome = async (/** @type {() => unknown} */ call) => {
	let value;
	try {
		value = call();
	} catch (error) {
		return { threw: show_error(error) };
	}
	if (!(value instanceof Promise)) return { returned: show_value(value) };
	try {
		return { resolved: show_value(await value) };
	} catch (error) {
		return { rejected: show_error(error) };
	}
};

/**
 * How `call` ended, an error shown by its class alone: a refused receiver's message is the
 * engine's on Node, where a private field's read refuses it, and the host's on fgro.
 */
const class_outcome = async (/** @type {() => unknown} */ call) => {
	const ended = /** @type {Record<string, any>} */ (await outcome(call));
	const [how, shown] = Object.entries(ended)[0] ?? [];
	return { [String(how)]: shown?.class ?? shown };
};

/** Each of `calls`, by its label, as it ended. */
const outcomes = async (
	/** @type {Record<string, () => unknown>} */ calls,
	/** @type {typeof outcome} */ end = outcome
) => {
	/** @type {Record<string, unknown>} */
	const ended = {};
	for (const [label, call] of Object.entries(calls)) ended[label] = await end(call);
	return ended;
};

/**
 * How a listing ended, iterated by hand: each entry, then how it ended, `done` or the
 * rejection, and what one more `next()` settles to.
 */
const drain = async (/** @type {AsyncIterator<unknown>} */ listing) => {
	/** @type {Array<unknown>} */
	const entries = [];
	for (let i = 0; i < 100; i++) {
		let result;
		try {
			result = await listing.next();
		} catch (error) {
			return { entries, rejected: show_error(error), after: show_value(await listing.next()) };
		}
		if (result.done) {
			return { entries, done: show_value(result.value), after: show_value(await listing.next()) };
		}
		entries.push(show_value(result.value));
	}
	return { entries, unfinished: true };
};

/** How a listing `call` made ended: whether the call returned, then how it drained. */
const listed = async (/** @type {() => any} */ call) => {
	let listing;
	try {
		listing = call();
	} catch (error) {
		return { threw: show_error(error) };
	}
	return { returned: typeof listing, ...(await drain(listing)) };
};

/** The paths beneath the tree each read is asked for. */
const PATHS = [
	'a.txt',
	'sub',
	'sub/b.txt',
	'link_file',
	'link_dir',
	'link_dir/b.txt',
	'link_up',
	'escape',
	'absolute',
	'dangling',
	'missing',
	'missing/x',
	'a.txt/',
	'a.txt/x',
	'sub/',
	'sub/.',
	'sub/..',
	'sub/../a.txt',
	'link_dir/..',
	'.',
	'',
	'..',
	'../fs/a.txt',
	'sub/../../x',
	'/etc/hostname',
	'./a.txt',
	'sub//b.txt',
	'empty.txt',
	'large.txt',
	'mixed.bin',
	'order',
	'order/link'
];

/** The symlinks the case's runner makes in `scratch/`, which no op here can make on fgro. */
const LINKS = ['dangling', 'to_sub', 'to_file', 'escape', 'absolute'];

/**
 * A handle on `scratch/`, which the case's runner made empty but for its symlinks and the run
 * may write, holding `a.txt` and `sub/b.txt`, made through the handle.
 * @param {typeof import('../../../../../../src/lib/fs.ts')} ns
 */
const open_scratch = async (ns) => {
	const dir = await ns.fs.open_dir(`${project}/scratch`);
	await dir.mkdir('sub');
	await dir.write('a.txt', 'a');
	await dir.write('sub/b.txt', 'b');
	return dir;
};

/**
 * The tree beneath `dir`, each entry's path, kind, and a file's text, a symlink not followed,
 * so what an op left is shown with how it ended.
 * @param {import('../../../../../../src/lib/fs.ts').FsDir} dir
 * @param {string} [at]
 * @returns {Promise<Array<string>>}
 */
const tree = async (dir, at = '') => {
	/** @type {Array<string>} */
	const shown = [];
	for await (const entry of dir.list(at)) {
		const path = at === '' ? entry.name : `${at}/${entry.name}`;
		if (entry.kind === 'directory') {
			shown.push(`${path}/`, ...(await tree(dir, path)));
		} else if (entry.kind === 'file') {
			shown.push(`${path}: ${await dir.read_text(path)}`);
		} else {
			shown.push(`${path} (${entry.kind})`);
		}
	}
	return shown;
};

/**
 * Each of `calls` through `dir`, by its label, in order, as it ended, and the tree after it.
 * @param {import('../../../../../../src/lib/fs.ts').FsDir} dir
 * @param {Record<string, () => unknown>} calls
 */
const steps = async (dir, calls) => {
	/** @type {Record<string, unknown>} */
	const ended = {};
	for (const [label, call] of Object.entries(calls)) {
		ended[label] = { ...(await outcome(call)), tree: await tree(dir) };
	}
	return ended;
};

/**
 * The scenarios of the fs module, by name, each run in the project the fixtures are, whose
 * `fs/` directory holds the tree.
 * @type {Record<string, (ns: typeof import('../../../../../../src/lib/fs.ts'), rest: Array<string>) => Promise<unknown> | unknown>}
 */
const SCENARIOS = {
	// the namespace, the root, its functions, a handle, and the handles' prototypes
	shape: async (ns) => {
		const { fs } = ns;
		const dir = fs.project;
		const read_only = dir.readonly();
		const dir_prototype = Object.getPrototypeOf(dir);
		const read_prototype = Object.getPrototypeOf(read_only);
		const info = await dir.stat('fs/a.txt');
		const listing = dir.list('fs/sub');
		const result = /** @type {any} */ (await listing.next());
		const bytes = await dir.read('fs/a.txt');
		return {
			namespace: describe(ns),
			fs: describe(fs),
			fs_prototype: Object.getPrototypeOf(fs),
			functions: ['open_dir', 'real_path', 'temp_dir'].map((name) => {
				const fn = /** @type {any} */ (fs)[name];
				return { name, shape: describe(fn), length: fn.length, name_of: fn.name };
			}),
			project_is_the_cwd: relative(dir.path),
			dir: describe(dir),
			read_only: describe(read_only),
			extends: Object.getPrototypeOf(dir_prototype) === read_prototype,
			read_prototype_prototype: Object.getPrototypeOf(read_prototype),
			methods: [
				['stat', read_prototype],
				['stat_sync', read_prototype],
				['exists', read_prototype],
				['exists_sync', read_prototype],
				['dir', read_prototype],
				['read', read_prototype],
				['read_text', read_prototype],
				['read_if_exists', read_prototype],
				['read_text_if_exists', read_prototype],
				['read_text_sync', read_prototype],
				['read_link', read_prototype],
				['list', read_prototype],
				['dir', dir_prototype],
				['readonly', dir_prototype]
			].map(([name, prototype]) => {
				const method = /** @type {any} */ (prototype)[/** @type {string} */ (name)];
				return {
					name,
					length: method.length,
					name_of: method.name,
					frozen: Object.isFrozen(method)
				};
			}),
			same_root: fs.project === fs.project,
			thenable: 'then' in dir || 'then' in fs,
			info_keys: Object.keys(info ?? {}),
			info_null_prototype: Object.getPrototypeOf(info) === null,
			listing: describe(listing),
			listing_methods: ['next', 'return', 'throw'].map((name) => {
				const method = /** @type {any} */ (listing)[name];
				return { name, length: method.length, name_of: method.name };
			}),
			listing_iterates_itself: listing[Symbol.asyncIterator]() === listing,
			listing_iterator_name: listing[Symbol.asyncIterator].name,
			result_keys: Object.keys(result),
			result_null_prototype: Object.getPrototypeOf(result) === null,
			entry: describe(result.value),
			entry_object_prototype: Object.getPrototypeOf(result.value) === Object.prototype,
			bytes: describe(bytes),
			property_write: thrown(() => {
				/** @type {any} */ (dir).path = '/';
			}),
			fs_write: thrown(() => {
				/** @type {any} */ (fs).project = null;
			}),
			method_write: thrown(() => {
				/** @type {any} */ (read_prototype).stat = null;
			}),
			new_stat: thrown(() => new /** @type {any} */ (dir.stat)('fs/a.txt')),
			read_only_writes: 'write' in read_only
		};
	},
	// `stat`, `stat_sync`, `exists`, and `exists_sync` of each path beneath the tree's handle,
	// and `stat` without following a final symlink
	stat: async (ns) => {
		const tree = ns.fs.project.dir('fs');
		/** @type {Record<string, unknown>} */
		const ended = {};
		for (const path of PATHS) {
			ended[path] = await outcomes({
				stat: () => tree.stat(path),
				stat_sync: () => tree.stat_sync(path),
				exists: () => tree.exists(path),
				exists_sync: () => tree.exists_sync(path),
				keep: () => tree.stat(path, { follow: false })
			});
		}
		return ended;
	},
	// each whole-file read and `read_link` of each path beneath the tree's handle
	read: async (ns) => {
		const tree = ns.fs.project.dir('fs');
		/** @type {Record<string, unknown>} */
		const ended = {};
		for (const path of PATHS) {
			ended[path] = await outcomes({
				read: () => tree.read(path),
				read_text: () => tree.read_text(path),
				read_if_exists: () => tree.read_if_exists(path),
				read_text_if_exists: () => tree.read_text_if_exists(path),
				read_text_sync: () => tree.read_text_sync(path),
				read_link: () => tree.read_link(path)
			});
		}
		// a final symlink named with a trailing `/` is followed, so no symlink is left to read
		ended.links_followed = await outcomes({
			to_a_directory: () => tree.read_link('link_dir/'),
			out: () => tree.read_link('escape/'),
			to_a_file: () => tree.read_link('link_file/')
		});
		// the bytes are the program's own: each read a buffer of its own
		const first = await tree.read('a.txt');
		const second = await tree.read('a.txt');
		first[0] = 0;
		ended.own_bytes = {
			distinct: first.buffer !== second.buffer,
			whole_buffer: first.byteOffset === 0 && first.buffer.byteLength === first.length,
			second: Array.from(second)
		};
		return ended;
	},
	// listings of each directory beneath the tree's handle, each failure at the first `next()`
	list: async (ns) => {
		const tree = ns.fs.project.dir('fs');
		/** @type {Record<string, unknown>} */
		const ended = {};
		for (const path of PATHS) ended[path] = await listed(() => tree.list(path));
		ended.no_path = await listed(() => tree.list());
		ended.undefined_path = await listed(() => tree.list(undefined));
		// a listing of the handle's own directory, by `for await`
		/** @type {Array<unknown>} */
		const iterated = [];
		for await (const entry of tree.list('order')) iterated.push(show_value(entry));
		ended.for_await = iterated;
		return ended;
	},
	// the iterator `list` returns, used every way: its work waits for its first `next()`, a
	// `return` or a `throw` ends it, as an async generator's does
	listing: async (ns) => {
		const tree = ns.fs.project.dir('fs');
		/** @type {Record<string, unknown>} */
		const ended = {};
		const returned_first = tree.list('missing');
		ended.returned_first = {
			returned: show_value(await returned_first.return(5)),
			after: show_value(await returned_first.next())
		};
		const thrown_first = tree.list('missing');
		const thrown = new Error('thrown in');
		ended.thrown_first = {
			thrown: await thrown_first.throw(thrown).then(show_value, (error) => error === thrown),
			after: show_value(await thrown_first.next())
		};
		const midway = tree.list('sub');
		ended.returned_midway = {
			first: show_value(await midway.next()),
			returned: show_value(await midway.return(undefined)),
			after: show_value(await midway.next())
		};
		const thrown_midway = tree.list('order');
		await thrown_midway.next();
		ended.thrown_midway = {
			thrown: await thrown_midway.throw(thrown).then(show_value, (error) => error === thrown),
			after: show_value(await thrown_midway.next())
		};
		// a `break` and a throw out of `for await` end the listing through its `return`
		const broken = tree.list('order');
		/** @type {Array<unknown>} */
		const before_break = [];
		for await (const entry of broken) {
			before_break.push(show_value(entry));
			if (before_break.length === 2) break;
		}
		ended.broken = { before_break, after: show_value(await broken.next()) };
		const thrown_out = tree.list('order');
		try {
			for await (const entry of thrown_out) {
				void entry;
				throw thrown;
			}
		} catch (error) {
			ended.thrown_out = { caught: error === thrown, after: show_value(await thrown_out.next()) };
		}
		// calls made before any settles settle in order
		const raced = tree.list('sub');
		ended.raced = (await Promise.all([raced.next(), raced.next(), raced.next()])).map(show_value);
		// two listings of one directory are two iterations
		const one = tree.list('sub');
		const two = tree.list('sub');
		ended.two = [show_value(await one.next()), show_value(await two.next())];
		return ended;
	},
	// a unix socket, the case's runner's: no read reads it, `stat` and a listing show it
	special: async (ns) => {
		const tree = ns.fs.project.dir('fs');
		return {
			reads: await outcomes({
				read: () => tree.read('socket'),
				read_text: () => tree.read_text('socket'),
				read_if_exists: () => tree.read_if_exists('socket'),
				read_text_if_exists: () => tree.read_text_if_exists('socket'),
				read_text_sync: () => tree.read_text_sync('socket'),
				read_link: () => tree.read_link('socket'),
				stat: () => tree.stat('socket'),
				directory: () => tree.read('sub'),
				directory_text_sync: () => tree.read_text_sync('sub/')
			}),
			listed: await listed(() => tree.list())
		};
	},
	// handles narrowed from the project's and from each other, and what each reads
	dir: async (ns) => {
		const { fs } = ns;
		const tree = fs.project.dir('fs');
		const read_only = tree.readonly();
		/** @type {Record<string, unknown>} */
		const ended = {};
		for (const path of PATHS) {
			ended[path] = await outcomes({
				dir: () => tree.dir(path),
				read_only: () => read_only.dir(path)
			});
		}
		const sub = tree.dir('link_dir');
		ended.through_a_link = await outcomes({
			path: () => sub.path,
			read: () => sub.stat('b.txt'),
			up: () => sub.stat('..'),
			narrower: () => sub.dir('.').stat('b.txt')
		});
		ended.read_only_of_read_only = await outcome(() => read_only.dir('sub').dir('.'));
		return ended;
	},
	// directories opened by absolute path inside the project
	open_dir: async (ns) => {
		const { fs } = ns;
		const root = fs.project.path;
		return outcomes({
			project: () => fs.open_dir(root),
			tree: () => fs.open_dir(`${root}/fs`),
			sub: () => fs.open_dir(`${root}/fs/sub/`),
			dotted: () => fs.open_dir(`${root}/fs/sub/../sub/.`),
			// a `..` resolves after the name before it, as `realpath` resolves it
			up_from_missing: () => fs.open_dir(`${root}/fs/missing/..`),
			up_from_a_file: () => fs.open_dir(`${root}/fs/a.txt/..`),
			// a `..` above the project, back into it
			reentering: () => fs.open_dir(`${root}/fs/../../${name_of(root)}/fs/sub`),
			through_a_link: () => fs.open_dir(`${root}/fs/link_dir`),
			then_read: async () => (await fs.open_dir(`${root}/fs`)).stat('sub/b.txt'),
			file: () => fs.open_dir(`${root}/fs/a.txt`),
			link_to_a_file: () => fs.open_dir(`${root}/fs/link_file`),
			missing: () => fs.open_dir(`${root}/fs/missing`),
			dangling: () => fs.open_dir(`${root}/fs/dangling`),
			relative: () => fs.open_dir('fs'),
			empty: () => fs.open_dir(''),
			not_a_string: () => fs.open_dir(/** @type {any} */ (1)),
			nul: () => fs.open_dir(`${root}/a\0b`),
			lone_surrogate: () => fs.open_dir(`${root}/\uD800`)
		});
	},
	// a directory outside every root the run reads: refused on fgro, also when nothing is
	// there, where Node opens what's there
	open_dir_outside: async (ns) => {
		const { fs } = ns;
		return outcomes({
			missing: () => fs.open_dir('/no_such_directory_of_the_fixtures')
		});
	},
	// real paths inside the project
	real_path: async (ns) => {
		const { fs } = ns;
		const root = fs.project.path;
		return outcomes({
			file: () => fs.real_path(`${root}/fs/a.txt`),
			link: () => fs.real_path(`${root}/fs/link_file`),
			through_a_link: () => fs.real_path(`${root}/fs/link_dir/b.txt`),
			climbing_link: () => fs.real_path(`${root}/fs/link_up`),
			dir: () => fs.real_path(`${root}/fs/sub/`),
			dangling: () => fs.real_path(`${root}/fs/dangling`),
			up_from_missing: () => fs.real_path(`${root}/fs/missing/..`),
			up_from_a_file: () => fs.real_path(`${root}/fs/a.txt/..`),
			// a symlink to a file, so the `..` after it finds no directory
			up_from_a_link_to_a_file: () => fs.real_path(`${root}/fs/link_up/..`),
			reentering: () => fs.real_path(`${root}/../${name_of(root)}/fs/a.txt`),
			missing: () => fs.real_path(`${root}/fs/missing`),
			through_a_file: () => fs.real_path(`${root}/fs/a.txt/x`),
			relative: () => fs.real_path('fs/a.txt'),
			not_a_string: () => fs.real_path(/** @type {any} */ (null))
		});
	},
	// paths that are none, each a `TypeError`, thrown by a sync op and rejected by an async one
	arguments: async (ns) => {
		const tree = ns.fs.project.dir('fs');
		/** @type {Array<[string, unknown]>} */
		const paths = [
			['a number', 1],
			['undefined', undefined],
			['null', null],
			['a boxed string', new String('a.txt')],
			['a symbol', Symbol('a.txt')],
			['a NUL', 'a\0.txt'],
			['a lone surrogate', 'a\uD800.txt']
		];
		/** @type {Record<string, unknown>} */
		const ended = {};
		for (const [what, path] of paths) {
			const given = /** @type {any} */ (path);
			ended[what] = await outcomes({
				stat: () => tree.stat(given),
				stat_sync: () => tree.stat_sync(given),
				exists: () => tree.exists(given),
				exists_sync: () => tree.exists_sync(given),
				dir: () => tree.dir(given),
				read: () => tree.read(given),
				read_text_sync: () => tree.read_text_sync(given),
				read_link: () => tree.read_link(given),
				list: () => tree.list(given).next()
			});
		}
		return ended;
	},
	// a method called on what isn't a handle of its kind
	receiver: async (ns) => {
		const { fs } = ns;
		const dir = fs.project;
		const read_only = dir.readonly();
		const read_prototype = /** @type {any} */ (Object.getPrototypeOf(read_only));
		const dir_prototype = /** @type {any} */ (Object.getPrototypeOf(dir));
		const listing_prototype = /** @type {any} */ (Object.getPrototypeOf(dir.list()));
		/** @type {Array<[string, unknown]>} */
		const receivers = [
			['undefined', undefined],
			['an object', {}],
			['a look-alike', Object.create(dir_prototype)],
			['a proxy', new Proxy(dir, {})],
			['the prototype', read_prototype]
		];
		/** @type {Record<string, unknown>} */
		const ended = {};
		for (const [what, receiver] of receivers) {
			ended[what] = await outcomes(
				{
					stat: () => Reflect.apply(read_prototype.stat, receiver, ['fs']),
					stat_sync: () => Reflect.apply(read_prototype.stat_sync, receiver, ['fs']),
					dir: () => Reflect.apply(read_prototype.dir, receiver, ['fs']),
					readonly: () => Reflect.apply(dir_prototype.readonly, receiver, []),
					read: () => Reflect.apply(read_prototype.read, receiver, ['fs/a.txt']),
					read_text_sync: () =>
						Reflect.apply(read_prototype.read_text_sync, receiver, ['fs/a.txt']),
					read_link: () => Reflect.apply(read_prototype.read_link, receiver, ['fs/link_file']),
					// the receiver is read at the first `next()`, as the Node module's is
					list: () => Reflect.apply(read_prototype.list, receiver, ['fs']).next(),
					next: () => Reflect.apply(listing_prototype.next, receiver, []),
					return: () => Reflect.apply(listing_prototype.return, receiver, [1]),
					throw: () => Reflect.apply(listing_prototype.throw, receiver, [1])
				},
				class_outcome
			);
		}
		// the write ops' own `dir` and `readonly` refuse a read-only handle
		ended.read_only = await outcomes(
			{
				dir: () => Reflect.apply(dir_prototype.dir, read_only, ['fs']),
				readonly: () => Reflect.apply(dir_prototype.readonly, read_only, [])
			},
			class_outcome
		);
		// the root's functions take no receiver
		const { open_dir } = fs;
		ended.detached = await outcome(() => open_dir(dir.path));
		return ended;
	},
	// `write` of each kind of path, data, and option, through a handle on `scratch/`
	write: async (ns) => {
		const dir = await open_scratch(ns);
		return steps(dir, {
			new: () => dir.write('new.txt', 'new'),
			replaces: () => dir.write('a.txt', 'replaced'),
			create_new: () => dir.write('made.txt', 'made', { create_new: true }),
			create_new_again: () => dir.write('made.txt', 'again', { create_new: true }),
			create_new_truthy: () =>
				dir.write('made.txt', 'again', /** @type {any} */ ({ create_new: 1 })),
			create_new_falsy: () =>
				dir.write('made.txt', 'falsy', /** @type {any} */ ({ create_new: 0 })),
			create_new_on_a_directory: () => dir.write('sub', 'x', { create_new: true }),
			create_new_on_its_own_directory: () => dir.write('', 'x', { create_new: true }),
			create_new_on_a_dangling_link: () => dir.write('dangling', 'x', { create_new: true }),
			a_directory: () => dir.write('sub', 'x'),
			a_directory_slashed: () => dir.write('sub/', 'x'),
			its_own_directory: () => dir.write('', 'x'),
			dot: () => dir.write('.', 'x'),
			a_trailing_slash: () => dir.write('slashed.txt/', 'x'),
			a_missing_parent: () => dir.write('missing/x.txt', 'x'),
			through_a_file: () => dir.write('a.txt/x', 'x'),
			through_a_link_to_a_file: () => dir.write('to_file', 'linked'),
			through_a_link_to_a_directory: () => dir.write('to_sub/c.txt', 'c'),
			up_and_back: () => dir.write('sub/../up.txt', 'up'),
			a_link_out: () => dir.write('escape', 'x'),
			an_absolute_link: () => dir.write('absolute', 'x'),
			above: () => dir.write('../x.txt', 'x'),
			absolute: () => dir.write('/x.txt', 'x'),
			bytes: () => dir.write('bytes.bin', new Uint8Array([104, 105])),
			part_of_a_buffer: () =>
				dir.write('part.bin', new Uint8Array([0, 104, 105, 0]).subarray(1, 3)),
			empty: () => dir.write('empty.txt', ''),
			lone_surrogate: async () => {
				await dir.write('surrogate.txt', 'a\uD800b');
				return Array.from(await dir.read('surrogate.txt')).join(' ');
			},
			mode: async () => {
				await dir.write('mode.txt', 'x', { mode: 0o640 });
				return (await dir.stat('mode.txt'))?.mode;
			},
			// the sticky bit, which a write leaves, where it clears a setuid or setgid bit itself
			mode_masked: async () => {
				await dir.write('masked.txt', 'x', { mode: 0o1640 });
				return (await dir.stat('masked.txt'))?.mode;
			},
			mode_undefined: () => dir.write('undefined_mode.txt', 'x', { mode: undefined }),
			// the dangling link's target, made beneath the handle
			a_dangling_link: () => dir.write('dangling', 'through the link'),
			...Object.fromEntries(
				/** @type {Array<[string, unknown]>} */ ([
					['a number', 1],
					['null', null],
					['an array', [104]],
					['a Uint16Array', new Uint16Array([104])],
					['a DataView', new DataView(new ArrayBuffer(1))],
					['an ArrayBuffer', new ArrayBuffer(1)],
					['a boxed string', new String('x')]
				]).map(([what, data]) => [
					`data ${what}`,
					() => dir.write('data.txt', /** @type {any} */ (data))
				])
			),
			...Object.fromEntries(
				/** @type {Array<[string, unknown]>} */ ([
					['a string', '755'],
					['a fraction', 1.5],
					['negative', -1],
					['too large', 0o10000],
					['NaN', NaN],
					['null', null]
				]).map(([what, mode]) => [
					`mode ${what}`,
					() => dir.write('mode_refused.txt', 'x', /** @type {any} */ ({ mode }))
				])
			)
		});
	},
	// `mkdir` of each kind of path and option, through a handle on `scratch/`
	mkdir: async (ns) => {
		const dir = await open_scratch(ns);
		return steps(dir, {
			new: () => dir.mkdir('d'),
			again: () => dir.mkdir('d'),
			exists_ok: () => dir.mkdir('d', { exists_ok: true }),
			recursive_existing: () => dir.mkdir('d', { recursive: true }),
			recursive: () => dir.mkdir('r/s/t', { recursive: true }),
			recursive_again: () => dir.mkdir('r/s/t', { recursive: true }),
			recursive_truthy: () => dir.mkdir('n/o', /** @type {any} */ ({ recursive: 1 })),
			recursive_falsy: () => dir.mkdir('m/o', /** @type {any} */ ({ recursive: 0 })),
			slashed: () => dir.mkdir('slashed/'),
			a_missing_parent: () => dir.mkdir('missing/x'),
			a_file: () => dir.mkdir('a.txt'),
			a_file_exists_ok: () => dir.mkdir('a.txt', { exists_ok: true }),
			a_file_recursive: () => dir.mkdir('a.txt', { recursive: true }),
			through_a_file: () => dir.mkdir('a.txt/x'),
			through_a_file_recursive: () => dir.mkdir('a.txt/x/y', { recursive: true }),
			its_own_directory: () => dir.mkdir(''),
			its_own_directory_exists_ok: () => dir.mkdir('', { exists_ok: true }),
			its_own_directory_recursive: () => dir.mkdir('.', { recursive: true }),
			a_dangling_link: () => dir.mkdir('dangling'),
			a_dangling_link_exists_ok: () => dir.mkdir('dangling', { exists_ok: true }),
			a_link_to_a_directory: () => dir.mkdir('to_sub'),
			a_link_to_a_directory_exists_ok: () => dir.mkdir('to_sub', { exists_ok: true }),
			a_link_to_a_directory_recursive: () => dir.mkdir('to_sub', { recursive: true }),
			a_link_to_a_file_exists_ok: () => dir.mkdir('to_file', { exists_ok: true }),
			// Node's recursive `mkdir` fails with what its `stat` of the path finds
			a_dangling_link_recursive: () => dir.mkdir('dangling', { recursive: true }),
			// the Node module's `exists_ok` check stats the path as given, its `/` kept
			a_file_slashed_exists_ok: () => dir.mkdir('a.txt/', { exists_ok: true }),
			a_file_slashed_dot_exists_ok: () => dir.mkdir('a.txt/.', { exists_ok: true }),
			a_link_to_a_file_slashed_exists_ok: () => dir.mkdir('to_file/', { exists_ok: true }),
			a_dangling_link_slashed_exists_ok: () => dir.mkdir('dangling/', { exists_ok: true }),
			a_file_slashed_recursive_exists_ok: () =>
				dir.mkdir('a.txt/', { recursive: true, exists_ok: true }),
			// Node's recursive `mkdir` makes the path without its `/`, an entry there no directory
			a_file_slashed_recursive: () => dir.mkdir('a.txt/', { recursive: true }),
			// the Node module's confinement follows a final symlink first, failing as it does
			a_looping_link: () => dir.mkdir('loop'),
			a_link_through_a_file: () => dir.mkdir('thru'),
			through_a_link: () => dir.mkdir('to_sub/new'),
			through_a_link_recursive: () => dir.mkdir('to_sub/x/y', { recursive: true }),
			a_link_out: () => dir.mkdir('escape'),
			a_link_out_exists_ok: () => dir.mkdir('escape', { exists_ok: true }),
			an_absolute_link_exists_ok: () => dir.mkdir('absolute', { exists_ok: true }),
			through_a_link_out: () => dir.mkdir('escape/x', { recursive: true }),
			above: () => dir.mkdir('../x'),
			above_recursive: () => dir.mkdir('sub/../../x', { recursive: true }),
			through_a_dangling_link: () => dir.mkdir('dangling/x'),
			// the Node module's confinement resolves the link to the path it names, and makes that
			through_a_dangling_link_recursive: () => dir.mkdir('dangling/x', { recursive: true })
		});
	},
	// `remove` of each kind of path and option, through a handle on `scratch/`, the links last
	remove: async (ns) => {
		const dir = await open_scratch(ns);
		return steps(dir, {
			a_file: () => dir.remove('a.txt'),
			missing: () => dir.remove('a.txt'),
			missing_ok: () => dir.remove('a.txt', { missing_ok: true }),
			missing_ok_truthy: () => dir.remove('a.txt', /** @type {any} */ ({ missing_ok: 1 })),
			a_missing_parent_missing_ok: () => dir.remove('missing/x', { missing_ok: true }),
			a_full_directory: () => dir.remove('sub'),
			through_a_file: () => dir.write('f', 'f').then(() => dir.remove('f/x')),
			through_a_file_missing_ok: () => dir.remove('f/x', { missing_ok: true }),
			a_file_slashed: () => dir.remove('f/'),
			a_file_slashed_missing_ok: () => dir.remove('f/', { missing_ok: true }),
			a_file_recursive: () => dir.remove('f', { recursive: true }),
			an_empty_directory: () => dir.mkdir('e').then(() => dir.remove('e')),
			an_empty_directory_slashed: () => dir.mkdir('e').then(() => dir.remove('e/')),
			its_own_directory: () => dir.remove(''),
			its_own_directory_dotted: () => dir.remove('sub/..'),
			its_own_directory_missing_ok: () => dir.remove('.', { missing_ok: true }),
			above: () => dir.remove('../x'),
			absolute: () => dir.remove('/x'),
			missing_recursive: () => dir.remove('nothing', { recursive: true }),
			missing_recursive_missing_ok: () =>
				dir.remove('nothing', { recursive: true, missing_ok: true }),
			a_link_to_a_directory_slashed: () => dir.remove('to_sub/'),
			a_link_to_a_directory_slashed_recursive: () => dir.remove('to_sub/', { recursive: true }),
			through_a_link: () => dir.remove('to_sub/b.txt'),
			a_tree: async () => {
				await dir.mkdir('t/u/v', { recursive: true });
				await dir.write('t/f', 'f');
				await dir.write('t/u/v/g', 'g');
				return dir.remove('t/', { recursive: true });
			},
			a_tree_truthy: async () => {
				await dir.mkdir('t/u', { recursive: true });
				return dir.remove('t', /** @type {any} */ ({ recursive: 'yes' }));
			},
			// each link removed, never its target
			a_link_to_a_directory: () => dir.remove('to_sub'),
			a_link_to_a_file: () => dir.remove('to_file', { recursive: true }),
			a_dangling_link: () => dir.remove('dangling'),
			a_link_out: () => dir.remove('escape'),
			an_absolute_link: () => dir.remove('absolute')
		});
	},
	// `rename` of each kind of path, through a handle on `scratch/`, the links last
	rename: async (ns) => {
		const dir = await open_scratch(ns);
		return steps(dir, {
			a_file: () => dir.rename('a.txt', 'b.txt'),
			over_a_file: () => dir.write('c.txt', 'c').then(() => dir.rename('b.txt', 'c.txt')),
			into_a_directory: () => dir.rename('c.txt', 'sub/c.txt'),
			back_up: () => dir.rename('sub/c.txt', 'sub/../c.txt'),
			missing: () => dir.rename('missing', 'x'),
			a_missing_destination_parent: () => dir.rename('c.txt', 'missing/x'),
			both_missing: () => dir.rename('missing', 'missing/x'),
			through_a_file_from: () => dir.rename('c.txt/x', 'x'),
			through_a_file_to: () => dir.rename('sub', 'c.txt/x'),
			missing_through_a_file_to: () => dir.rename('missing', 'c.txt/x'),
			a_file_slashed: () => dir.rename('c.txt/', 'x'),
			a_file_over_a_directory: () => dir.mkdir('e').then(() => dir.rename('c.txt', 'e')),
			a_directory_over_a_file: () => dir.rename('e', 'c.txt'),
			a_directory_over_a_full_one: () => dir.rename('e', 'sub'),
			a_directory_over_an_empty_one: () => dir.mkdir('e2').then(() => dir.rename('e', 'e2')),
			a_directory_into_itself: () => dir.rename('sub', 'sub/inner'),
			a_directory_slashed: () => dir.rename('e2/', 'e3'),
			its_own_directory_from: () => dir.rename('', 'x'),
			its_own_directory_to: () => dir.rename('c.txt', '.'),
			its_own_directory_dotted: () => dir.rename('sub/..', 'x'),
			above_from: () => dir.rename('../x', 'y'),
			above_to: () => dir.rename('c.txt', '../y'),
			absolute_to: () => dir.rename('c.txt', '/y'),
			through_a_link_out_from: () => dir.rename('escape/x', 'y'),
			through_a_link_out_to: () => dir.rename('c.txt', 'escape/x'),
			the_same: () => dir.rename('c.txt', 'c.txt'),
			// a symlink named with a trailing `/`: the kernel refuses it, naming its destination
			// since `lstat` with the slash finds the directory it leads to
			a_link_slashed: () => dir.rename('to_sub/', 'moved'),
			through_a_link: () => dir.rename('to_sub/b.txt', 'b.txt'),
			// each link moved, never its target
			a_link_to_a_directory: () => dir.rename('to_sub', 'moved_to_sub'),
			a_dangling_link: () => dir.rename('dangling', 'moved_dangling'),
			a_link_out: () => dir.rename('escape', 'moved_escape'),
			an_absolute_link: () => dir.rename('absolute', 'sub/moved_absolute')
		});
	},
	// paths and options that are none for each op that writes, each a `TypeError`
	write_arguments: async (ns) => {
		const dir = await open_scratch(ns);
		/** @type {Array<[string, unknown]>} */
		const paths = [
			['a number', 1],
			['undefined', undefined],
			['a boxed string', new String('a.txt')],
			['a NUL', 'a\0.txt'],
			['a lone surrogate', 'a\uD800.txt']
		];
		/** @type {Record<string, unknown>} */
		const ended = {};
		for (const [what, path] of paths) {
			const given = /** @type {any} */ (path);
			ended[what] = await outcomes({
				write: () => dir.write(given, 'x'),
				mkdir: () => dir.mkdir(given),
				remove: () => dir.remove(given),
				rename_from: () => dir.rename(given, 'x'),
				rename_to: () => dir.rename('a.txt', given)
			});
		}
		// a path checked before the data, and the data before the options
		ended.order = await outcomes({
			path_then_data: () => dir.write(/** @type {any} */ (1), /** @type {any} */ (1)),
			data_then_mode: () =>
				dir.write('x', /** @type {any} */ (1), /** @type {any} */ ({ mode: 'x' })),
			from_then_to: () => dir.rename('../x', /** @type {any} */ (1)),
			own_then_options: () => dir.remove('', { recursive: true })
		});
		// a mode that isn't a number is refused by its type, its conversions never run
		let converted = false;
		ended.mode_object = await outcome(() =>
			dir.write('x', 'x', {
				mode: /** @type {any} */ ({
					valueOf() {
						converted = true;
						return 0o600;
					}
				})
			})
		);
		ended.mode_converted = converted;
		ended.getter = await outcome(() =>
			dir.mkdir('sub', {
				get exists_ok() {
					return true;
				}
			})
		);
		ended.tree = await tree(dir);
		return ended;
	},
	// what an op runs no code of: options given through a getter or a proxy
	options: async (ns) => {
		const tree = ns.fs.project.dir('fs');
		return outcomes({
			plain: () => tree.stat('link_file', { follow: false }),
			truthy: () => tree.stat('link_file', /** @type {any} */ ({ follow: 1 })),
			falsy: () => tree.stat('link_file', /** @type {any} */ ({ follow: 0 })),
			nullish: () => tree.stat('link_file', /** @type {any} */ ({ follow: null })),
			inherited: () => tree.stat('link_file', Object.create({ follow: false })),
			primitive: () => tree.stat('link_file', /** @type {any} */ (5)),
			trailing_slash: () => tree.stat('link_dir/', { follow: false }),
			getter: () =>
				tree.stat('link_file', {
					get follow() {
						return false;
					}
				}),
			proxy: () => tree.stat('link_file', new Proxy({ follow: false }, {}))
		});
	}
};

/**
 * @param {typeof import('../../../../../../src/lib/fs.ts')} ns
 * @param {Array<string>} rest - the scenario's name
 */
export default (ns, [name]) => {
	const scenario = SCENARIOS[/** @type {string} */ (name)];
	if (scenario === undefined) throw new Error(`no scenario ${name}`);
	project = ns.fs.project.path;
	return scenario(ns, []);
};
