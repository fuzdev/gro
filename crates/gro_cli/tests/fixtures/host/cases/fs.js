import { describe, thrown } from '../describe.js';

// The fs host module over the tree under `fs/`, a directory of the project: files, a
// subdirectory, and symlinks that stay inside, climb out, are absolute, and dangle. Each path
// a value or an error names is shown with the project's path as `{project}`, since the
// checkout's path is the machine's.

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

/** What an op returned or settled to, as data. */
const show_value = (/** @type {any} */ value) => {
	if (value === null || typeof value !== 'object') {
		return typeof value === 'string' ? relative(value) : value;
	}
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
	'sub//b.txt'
];

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
				dir: () => tree.dir(given)
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
					readonly: () => Reflect.apply(dir_prototype.readonly, receiver, [])
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
