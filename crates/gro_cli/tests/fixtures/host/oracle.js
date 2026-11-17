// The oracle of the host modules' differential cases: runs each case in `cases.json` under
// this Node on the Node implementation in `src/lib`, and writes what each prints to
// `expected.json` (`node oracle.js --write`) or checks the file against it
// (`node oracle.js --check`, exit 1 naming what differs). `expected.json` is never edited by
// hand: the `fgro` tests hold each builtin module to it, but where a case's `differs` names a
// value that differs by design.
//
// A case is a scenario in `cases/` named for its host module, run in a fresh process started
// in this directory with the case's arguments, a `{cwd}` among them replaced by that
// directory, and an environment holding the case's `env` and nothing else, never the
// developer's. It prints one line of JSON and nothing on stderr. Its `flags` are `fgro run`'s,
// the grants it runs with, and its `refused` the lines `fgro` prints for what its rules refuse,
// neither of which Node has. Its `sockets` are paths, relative to this directory, where a unix
// socket listens while it runs, made by its runner here and by `fgro`'s tests alike, since git
// holds no socket. Its `scratch` is a directory, relative to this one, that its runner here and
// `fgro`'s tests alike make empty before it runs, but for the symlinks its `links` name, by name
// with their targets, and remove after, so a case that writes starts from the same tree on both.

import { spawnSync } from 'node:child_process';
import {
	lstatSync,
	mkdirSync,
	readFileSync,
	realpathSync,
	rmSync,
	symlinkSync,
	writeFileSync
} from 'node:fs';
import { createServer } from 'node:net';
import { join } from 'node:path';

const dir = realpathSync(import.meta.dirname);
const expected_path = join(dir, 'expected.json');

/** @typedef {{name: string, module: string, args: Array<string>, env?: Record<string, string>, sockets?: Array<string>, scratch?: Scratch}} Case */
/** @typedef {{dir: string, links: Record<string, string>}} Scratch */

/** @type {Array<Case>} */
const cases = JSON.parse(readFileSync(join(dir, 'cases.json'), 'utf8'));

/** Removes the socket at `path` an earlier run left, and nothing else. */
const remove_socket = (/** @type {string} */ path) => {
	if (lstatSync(path, { throwIfNoEntry: false })?.isSocket()) rmSync(path);
};

/**
 * Listens on a unix socket at each of `paths`, relative to this directory.
 * @param {Array<string>} paths
 * @returns {Promise<() => Promise<void>>} what closes them and removes their files
 */
const listen = async (paths) => {
	/** @type {Array<[string, import('node:net').Server]>} */
	const servers = [];
	for (const path of paths) {
		const at = join(dir, path);
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
	const at = join(dir, scratch.dir);
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
	const close = await listen(test_case.sockets ?? []);
	if (test_case.scratch) reset_scratch(test_case.scratch, true);
	const args = test_case.args.map((arg) => (arg === '{cwd}' ? dir : arg));
	const result = spawnSync(process.execPath, ['on_node.js', test_case.module, ...args], {
		cwd: dir,
		env: { ...test_case.env },
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
for (const test_case of cases) results[test_case.name] = await run(test_case);

const actual = `${JSON.stringify(results, null, '\t')}\n`;

const mode = process.argv[2];
if (mode === '--write' && process.argv.length === 3) {
	writeFileSync(expected_path, actual);
} else if (mode === '--check' && process.argv.length === 3) {
	const expected = readFileSync(expected_path, 'utf8');
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
