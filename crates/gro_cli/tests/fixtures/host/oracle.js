// The oracle of the host modules' differential cases: runs each case in `cases.json` under
// this Node on the Node implementation in `src/lib`, and writes what each prints to
// `expected.json` (`node oracle.js --write`) or checks the file against it
// (`node oracle.js --check`, exit 1 naming what differs). `expected.json` is never edited by
// hand: the `fgro` tests hold each builtin module to it, but where a case's `differs` names a
// value that differs by design.
//
// A case is a scenario in `cases/` named for its host module, run in a fresh process started
// in this directory with the case's arguments, a `{cwd}` among them replaced by that
// directory. It prints one line of JSON and nothing on stderr.

import { spawnSync } from 'node:child_process';
import { readFileSync, realpathSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const dir = realpathSync(import.meta.dirname);
const expected_path = join(dir, 'expected.json');

/** @type {Array<{name: string, module: string, args: Array<string>}>} */
const cases = JSON.parse(readFileSync(join(dir, 'cases.json'), 'utf8'));

/**
 * Runs `test_case` under Node, returning its exit code and what it printed, parsed.
 * @param {{name: string, module: string, args: Array<string>}} test_case
 */
const run = (test_case) => {
	const args = test_case.args.map((arg) => (arg === '{cwd}' ? dir : arg));
	const result = spawnSync(process.execPath, ['on_node.js', test_case.module, ...args], {
		cwd: dir,
		env: {},
		encoding: 'utf8'
	});
	if (result.error || result.signal !== null || result.stderr !== '') {
		throw new Error(
			`the case ${test_case.name} didn't run cleanly under Node: ${result.error ?? result.signal ?? result.stderr}`
		);
	}
	return { code: result.status, output: JSON.parse(result.stdout) };
};

const actual = `${JSON.stringify(
	Object.fromEntries(cases.map((test_case) => [test_case.name, run(test_case)])),
	null,
	'\t'
)}\n`;

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
