// The oracle of the resolver's differential cases: builds the tree `cases.json` describes in a
// fresh temporary directory, resolves each case's specifier from its importer under this Node,
// and writes what Node gave to `expected.json` (`node oracle.js --write`) or checks the file
// against it (`node oracle.js --check`, exit 1 naming what differs). `expected.json` is never
// edited by hand: the resolver's own test builds the same tree and holds `fgro`'s resolution
// to it, but where a case's `departure` says why `fgro` refuses what Node resolves or resolves
// it to the URL the case's `fgro_url` names.
//
// Each importer is a module made in the tree that resolves a specifier with
// `import.meta.resolve` and then imports it, so a resolution naming no file fails as an import
// does. A case resolves to its URL relative to the tree's real directory, or fails with Node's
// error code.

import {
	mkdirSync,
	mkdtempSync,
	readFileSync,
	realpathSync,
	rmSync,
	symlinkSync,
	writeFileSync
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { pathToFileURL } from 'node:url';

// an `"imports"` target's empty segment resolves with a deprecation warning, which isn't output
process.noDeprecation = true;

const dir = realpathSync(import.meta.dirname);
const expected_path = join(dir, 'expected.json');

/** @typedef {{importer: string, specifier: string, fgro?: string, fgro_url?: string, departure?: string}} Case */
/** @typedef {{files: Record<string, string>, links: Record<string, string>, cases: Array<Case>}} Cases */

/** @type {Cases} */
const { files, links, cases } = JSON.parse(readFileSync(join(dir, 'cases.json'), 'utf8'));

/** What each importer is in the tree: it resolves and imports what it's asked to. */
const PROBE = `export const probe = async (specifier) => {
	let url;
	try {
		url = import.meta.resolve(specifier);
		await import(specifier);
	} catch (error) {
		return {error: error.code};
	}
	return {url};
};
`;

const root = realpathSync(mkdtempSync(join(tmpdir(), 'gro_resolution_')));
/** @type {Array<{importer: string, specifier: string, node: {url: string} | {error: string}}>} */
const results = [];
try {
	const importers = new Set(cases.map((test_case) => test_case.importer));
	for (const [path, content] of Object.entries(files)) {
		mkdirSync(dirname(join(root, path)), { recursive: true });
		writeFileSync(join(root, path), importers.has(path) ? PROBE : content);
	}
	for (const [path, target] of Object.entries(links)) {
		symlinkSync(target, join(root, path));
	}
	const root_url = `${pathToFileURL(root).href}/`;
	for (const { importer, specifier } of cases) {
		/** @type {{probe: (specifier: string) => Promise<{url: string} | {error: string}>}} */
		const { probe } = await import(pathToFileURL(join(root, importer)).href);
		const result = await probe(specifier);
		results.push({
			importer,
			specifier,
			node:
				'url' in result
					? {
							url: result.url.startsWith(root_url) ? result.url.slice(root_url.length) : result.url
						}
					: result
		});
	}
} finally {
	rmSync(root, { recursive: true, force: true });
}

const actual = `${JSON.stringify(results, null, '\t')}\n`;

const mode = process.argv[2];
if (mode === '--write' && process.argv.length === 3) {
	writeFileSync(expected_path, actual);
} else if (mode === '--check' && process.argv.length === 3) {
	const expected = readFileSync(expected_path, 'utf8');
	if (expected !== actual) {
		console.error(
			`${expected_path} isn't what Node resolves under ${process.version}. ` +
				`If the change is meant, regenerate it with \`node ${join(dir, 'oracle.js')} --write\` ` +
				`and review the diff. Node resolves:\n${actual}`
		);
		process.exitCode = 1;
	}
} else {
	console.error('usage: node oracle.js --write | --check');
	process.exitCode = 2;
}
