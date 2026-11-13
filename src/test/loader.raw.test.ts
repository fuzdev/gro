import { test, assert, onTestFinished } from 'vitest';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { spawn_with_loader_args } from '#lib/gro_helpers.ts';

import { LOADER_SOURCE_PATH } from './test_helpers.ts';

/*

Imports text files through the real loader in a child `node`, so the default export is checked
as a module evaluates it: exactly the file's text, never code.

*/

// each file's name and bytes, imported as text by its extension or with `?raw`
const FILES: Array<[name: string, contents: string | Uint8Array]> = [
	['interpolation.txt', 'a ${1 + 1} b ${(globalThis.__gro_raw_ran = true), ""} c'],
	['undefined_name.txt', '${undefined_name}'],
	['backticks.txt', 'a ` b \\` c ``'],
	['backslashes.txt', 'a \\ b \\n c \\u0041 \\x41 \\0 \\'],
	['line_separators.txt', 'a\u2028b\u2029c'],
	// UTF-8 can't encode a lone surrogate, so these are the bytes a CESU-8 writer would emit
	['lone_surrogate.txt', new Uint8Array([0x61, 0xed, 0xa0, 0x80, 0x62])],
	['bom.txt', '\ufeffbom'],
	['crlf.txt', 'a\r\nb\r\n\r\n'],
	['lone_cr.txt', 'a\rb\r'],
	['nul.txt', 'a\0b\x01c'],
	['empty.txt', ''],
	['large.txt', 'x'.repeat(1 << 20) + '${`\\'],
	['style.css', '.a::before { content: "${b}" } /* ` */\n'],
	['image.svg', '<svg><text>${c} `d` \\e</text></svg>\n']
];

test('a text import is exactly the file, never evaluated', () => {
	const dir = realpathSync(mkdtempSync(join(tmpdir(), 'gro_loader_raw_')));
	onTestFinished(() => rmSync(dir, { recursive: true, force: true }));
	for (const [name, contents] of FILES) writeFileSync(join(dir, name), contents);
	const imports = FILES.map(([name], i) => {
		const specifier = name.endsWith('.txt') ? `./${name}?raw` : `./${name}`;
		return `import f${i} from ${JSON.stringify(specifier)};`;
	});
	const script = join(dir, 'main.mjs');
	writeFileSync(
		script,
		`${imports.join('\n')}
		console.log(JSON.stringify({
			texts: [${FILES.map((_, i) => `f${i}`).join(', ')}],
			ran: globalThis.__gro_raw_ran === true
		}));\n`
	);

	const result = spawnSync(
		process.execPath,
		spawn_with_loader_args(LOADER_SOURCE_PATH, script, [], undefined),
		{ encoding: 'utf8', env: { PATH: '/usr/bin:/bin' }, maxBuffer: 1 << 26, timeout: 60_000 }
	);
	assert.strictEqual(result.status, 0, result.stderr);
	const { texts, ran } = JSON.parse(result.stdout) as { texts: Array<string>; ran: boolean };
	assert.strictEqual(ran, false, 'no file text ran as code');
	FILES.forEach(([name], i) => {
		assert.strictEqual(texts[i], readFileSync(join(dir, name), 'utf8'), name);
	});
});
