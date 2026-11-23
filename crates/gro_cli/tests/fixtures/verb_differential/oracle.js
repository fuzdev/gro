// The oracle of the verb's differential: runs each case in `cases.json` under this Node on the
// Node gro in `src/lib`, launched as `spawn_with_loader_args` launches it, and writes what each
// printed to `expected.json` (`node oracle.js --write`) or checks the file against it
// (`node oracle.js --check`, exit 1 naming what differs). `expected.json` is never edited by hand:
// `tests/verb_differential.rs` holds `fgro` to it, but where a case's `differs` names a declared
// difference.
//
// A case runs in a fresh copy of the project, its `files` written over it, beside a
// `node_modules` that links the checkout's, where the project's tools and Gro's peers are
// installed, in a directory under `/tmp`, whatever `TMPDIR` says, so no `package.json` above it is
// read and its path is short: both consoles break an inspected array whose string reaches
// `WRAP_BOUND` characters onto lines of their own, which `{root}` doesn't hide, and a root that
// would reach it fails before anything runs. Its environment holds `PATH`, for the tools' `node`, and a `HOME` that's an empty directory
// of its own, never the developer's. What it printed is normalized as the binary's test normalizes
// it: the project's root as `{root}`, gro's version as `{version}`, and each duration as `{ms}`.
//
// Node 26 deprecates `module.register()`, which the Node gro's launch calls, and warns on every
// run; that warning is Node's own about the launch, so the oracle turns it off by its code.

import { spawnSync } from 'node:child_process';
import {
	mkdirSync,
	mkdtempSync,
	readFileSync,
	realpathSync,
	rmSync,
	symlinkSync,
	writeFileSync
} from 'node:fs';
import { dirname, join } from 'node:path';
import { pathToFileURL } from 'node:url';

const dir = realpathSync(import.meta.dirname);
const expected_path = join(dir, 'expected.json');
const checkout = join(dir, '../../../../..');
const lib = join(checkout, 'src/lib');

// the length of a string at which Node's console breaks an inspected array holding it, and the
// longest path under a root the cases print in one, `gen`'s list of its genfiles
const WRAP_BOUND = 66;
const LONGEST_PRINTED = '/src/lib/w.gen.ts';

/** @typedef {{code: number, stdout: string, stderr: string}} Output */
/** @typedef {{name: string, args: Array<string>, files?: Record<string, string>}} Case */

/** @type {{project: Record<string, string>, cases: Array<Case>}} */
const { project, cases } = JSON.parse(readFileSync(join(dir, 'cases.json'), 'utf8'));

/** @type {string} */
const version = JSON.parse(readFileSync(join(checkout, 'package.json'), 'utf8')).version;

// `spawn_with_loader_args`'s launch, the loader registered by its file URL
const register = `data:text/javascript,${encodeURIComponent(
	`import {register} from "node:module"; register(${JSON.stringify(
		pathToFileURL(join(lib, 'loader.ts')).href
	)});`
)}`;
const launch = [
	'--disable-warning=DEP0205',
	'--import',
	register,
	'--experimental-import-meta-resolve',
	'-C',
	'development',
	join(lib, 'invoke.ts')
];

/**
 * `text` with the project's `root`, gro's version, and each duration normalized.
 * @param {string} text
 * @param {string} root
 */
const normalize = (text, root) =>
	text
		.replaceAll(root, '{root}')
		.replaceAll(version, '{version}')
		.replace(/\d[\d,]*(?:\.\d+)?ms/g, '{ms}');

/**
 * Runs `test_case` under Node in a fresh copy of the project.
 * @param {Case} test_case
 * @returns {Output}
 */
const run = (test_case) => {
	const scratch = realpathSync(mkdtempSync('/tmp/gro_vd_'));
	try {
		const root = join(scratch, 'p');
		if (root.length + LONGEST_PRINTED.length >= WRAP_BOUND) {
			throw new Error(
				`${root}: the project's root is too long for the cases' inspected arrays, which break ` +
					`onto lines of their own at ${WRAP_BOUND} characters, and \`{root}\` can't hide that`
			);
		}
		const home = join(scratch, 'home');
		mkdirSync(home);
		for (const [path, content] of Object.entries({ ...project, ...test_case.files })) {
			mkdirSync(dirname(join(root, path)), { recursive: true });
			writeFileSync(join(root, path), content);
		}
		symlinkSync(join(checkout, 'node_modules'), join(root, 'node_modules'));
		const result = spawnSync(process.execPath, [...launch, ...test_case.args], {
			cwd: root,
			env: { PATH: process.env.PATH ?? '', HOME: home },
			encoding: 'utf8',
			stdio: ['ignore', 'pipe', 'pipe'],
			timeout: 60_000
		});
		if (result.error || result.signal !== null || result.status === null) {
			throw new Error(
				`the case ${test_case.name} didn't run under Node: ${result.error ?? result.signal}`
			);
		}
		return {
			code: result.status,
			stdout: normalize(result.stdout, root),
			stderr: normalize(result.stderr, root)
		};
	} finally {
		rmSync(scratch, { recursive: true, force: true });
	}
};

/** @type {Record<string, Output>} */
const results = {};
for (const test_case of cases) results[test_case.name] = run(test_case);

const actual = `${JSON.stringify(results, null, '\t')}\n`;

const mode = process.argv[2];
if (mode === '--write' && process.argv.length === 3) {
	writeFileSync(expected_path, actual);
} else if (mode === '--check' && process.argv.length === 3) {
	const expected = readFileSync(expected_path, 'utf8');
	if (expected !== actual) {
		console.error(
			`${expected_path} isn't what the Node gro prints under ${process.version}. ` +
				`If the change is meant, regenerate it with \`node ${join(dir, 'oracle.js')} --write\` ` +
				`and review the diff. The Node gro prints:\n${actual}`
		);
		process.exitCode = 1;
	}
} else {
	console.error('usage: node oracle.js --write | --check');
	process.exitCode = 2;
}
