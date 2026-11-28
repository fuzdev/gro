// The oracle of the host modules' differential cases: runs each case in `cases.json` under
// this Node on the Node implementation in `src/lib`, and writes what each prints to
// `expected.json` (`node oracle.js --write`) or checks the file against it
// (`node oracle.js --check`, exit 1 naming what differs). `expected.json` is never edited by
// hand: the `fgro` tests hold each builtin module to it, but where a case's `differs` names a
// value that differs by design.
//
// A case is a scenario in `cases/` named for its host module, run in a fresh process started
// in a copy of this directory under the OS temp directory, its symlinks copied as they are, so
// what a case writes never lands in the checkout, with the case's arguments, a `{cwd}` among
// them replaced by that copy, and an environment holding the case's `env` and nothing else,
// never the developer's, each `{cwd}` in a value replaced by that copy too. It prints one line
// of JSON and nothing on stderr. Its `flags` are `fgro run`'s, the grants it runs with, and its
// `refused` the lines `fgro` prints for what its rules refuse, neither of which Node has. Its
// `sockets` are paths, relative to the directory it starts in, where a unix socket listens while
// it runs, made by its runner here and by `fgro`'s tests alike, since git holds no socket. Its
// `scratch` is a directory, relative to that one, that its runner here and `fgro`'s tests alike
// make empty before it runs, but for the symlinks its `links` name, by name with their targets,
// and remove after, so a case that writes starts from the same tree on both.
//
// A kernel box that refuses unix sockets, like the one `fgro test` runs its tools in, refuses
// making a case's socket with `EPERM`; `--check` skips that case there, naming it, and checks the
// rest, while `--write` never skips one.

import { spawnSync } from 'node:child_process';
import {
	cpSync,
	lstatSync,
	mkdirSync,
	mkdtempSync,
	readFileSync,
	realpathSync,
	rmSync,
	symlinkSync,
	writeFileSync
} from 'node:fs';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const dir = realpathSync(import.meta.dirname);
const expected_path = join(dir, 'expected.json');

// where every case runs, a copy of this directory, removed once the cases have run
const temp = realpathSync(mkdtempSync(join(tmpdir(), 'gro_host_')));
const work = join(temp, 'host');
cpSync(dir, work, { recursive: true, verbatimSymlinks: true });

/** @typedef {{name: string, module: string, args: Array<string>, env?: Record<string, string>, sockets?: Array<string>, scratch?: Scratch}} Case */
/** @typedef {{dir: string, links: Record<string, string>}} Scratch */

/** @type {Array<Case>} */
const cases = JSON.parse(readFileSync(join(dir, 'cases.json'), 'utf8'));

const mode = process.argv[2];

/** The cases `--check` skipped because their sockets couldn't be made, each with why. */
const skipped = new Map();

/** Removes the socket at `path` an earlier run left, and nothing else. */
const remove_socket = (/** @type {string} */ path) => {
	if (lstatSync(path, { throwIfNoEntry: false })?.isSocket()) rmSync(path);
};

/**
 * Listens on a unix socket at each of `paths`, relative to the copy.
 * @param {Array<string>} paths
 * @returns {Promise<() => Promise<void>>} what closes them and removes their files
 */
const listen = async (paths) => {
	/** @type {Array<[string, import('node:net').Server]>} */
	const servers = [];
	for (const path of paths) {
		const at = join(work, path);
		remove_socket(at);
		const server = createServer();
		await new Promise((resolve, reject) => {
			server.once('error', reject);
			server.listen(at, () => resolve(undefined));
		});
		servers.push([at, server]);
	}
	return async () => {
		for (const [at, server] of servers) {
			await new Promise((resolve) => server.close(() => resolve(undefined)));
			remove_socket(at);
		}
	};
};

/**
 * Makes `scratch`'s directory empty but for its links, or, without `make`, removes it.
 * @param {Scratch} scratch
 * @param {boolean} make
 */
const reset_scratch = (scratch, make) => {
	const at = join(work, scratch.dir);
	rmSync(at, { recursive: true, force: true });
	if (!make) return;
	mkdirSync(at);
	for (const [name, target] of Object.entries(scratch.links)) symlinkSync(target, join(at, name));
};

/**
 * Runs `test_case` under Node, returning its exit code and what it printed, parsed.
 * @param {Case} test_case
 */
const run = async (test_case) => {
	let close;
	try {
		close = await listen(test_case.sockets ?? []);
	} catch (error) {
		if (mode !== '--check' || /** @type {any} */ (error)?.code !== 'EPERM') throw error;
		skipped.set(test_case.name, String(error));
		return undefined;
	}
	if (test_case.scratch) reset_scratch(test_case.scratch, true);
	const args = test_case.args.map((arg) => (arg === '{cwd}' ? work : arg));
	const env = Object.fromEntries(
		Object.entries(test_case.env ?? {}).map(([name, value]) => [
			name,
			value.replaceAll('{cwd}', work)
		])
	);
	const result = spawnSync(process.execPath, [join(dir, 'on_node.js'), test_case.module, ...args], {
		cwd: work,
		env,
		encoding: 'utf8'
	});
	if (test_case.scratch) reset_scratch(test_case.scratch, false);
	await close();
	if (result.error || result.signal !== null || result.stderr !== '') {
		throw new Error(
			`the case ${test_case.name} didn't run cleanly under Node: ${result.error ?? result.signal ?? result.stderr}`
		);
	}
	return { code: result.status, output: JSON.parse(result.stdout) };
};

/** @type {Record<string, unknown>} */
const results = {};
try {
	for (const test_case of cases) {
		const result = await run(test_case);
		if (result !== undefined) results[test_case.name] = result;
	}
} finally {
	rmSync(temp, { recursive: true, force: true });
}

const actual = `${JSON.stringify(results, null, '\t')}\n`;

if (mode === '--write' && process.argv.length === 3) {
	writeFileSync(expected_path, actual);
} else if (mode === '--check' && process.argv.length === 3) {
	let expected = readFileSync(expected_path, 'utf8');
	if (skipped.size > 0) {
		const parsed = JSON.parse(expected);
		for (const [name, why] of skipped) {
			if (!(name in parsed)) throw new Error(`the skipped case ${name} isn't in ${expected_path}`);
			delete parsed[name];
			console.error(`skipped the case ${name}, whose socket couldn't be made here: ${why}`);
		}
		expected = `${JSON.stringify(parsed, null, '\t')}\n`;
	}
	if (expected !== actual) {
		console.error(
			`${expected_path} isn't what the Node implementation prints under ${process.version}. ` +
				`If the change is meant, regenerate it with \`node ${join(dir, 'oracle.js')} --write\` ` +
				`and review the diff. Node prints:\n${actual}`
		);
		process.exitCode = 1;
	}
} else {
	console.error('usage: node oracle.js --write | --check');
	process.exitCode = 2;
}
