import { describe, test, assert } from 'vitest';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

import { paths } from '#lib/paths.ts';

/*

`fgro` embeds some `src/lib/internal/` modules as they are in its builtin modules
(`crates/gro_runtime/src/bootstrap.rs`), where a non-ASCII byte fails the runtime's build, which
`gro check` doesn't run, and an import would link a module the bootstrap doesn't embed. This
reads the embedded paths from the bootstrap and checks each, so the failure shows here first.

*/

const BOOTSTRAP = join(paths.root, 'crates/gro_runtime/src/bootstrap.rs');

/**
 * The `src/lib/internal/` files the bootstrap embeds, relative to the repo's root.
 */
const embedded_internal_files = (): Array<string> =>
	Array.from(
		readFileSync(BOOTSTRAP, 'utf8').matchAll(
			/ascii_str_include!\(\s*"\.\.\/\.\.\/\.\.\/(src\/lib\/internal\/[^"]+)"\s*\)/g
		),
		(match) => match[1]!
	);

describe('the src/lib/internal modules fgro embeds', () => {
	test('the bootstrap embeds at least one', () => {
		assert.include(embedded_internal_files(), 'src/lib/internal/records.ts');
	});

	test('each is 7-bit ASCII and imports nothing', () => {
		for (const file of embedded_internal_files()) {
			const source = readFileSync(join(paths.root, file), 'utf8');
			const non_ascii = Array.from(source).find((char) => char.charCodeAt(0) > 0x7f);
			assert.isUndefined(non_ascii, `${file}: non-ASCII ${JSON.stringify(non_ascii)}`);
			assert.notMatch(source, /^\s*(import|export)\b[^;]*\bfrom\s/m, `${file} imports`);
			assert.notMatch(source, /^\s*import\s*['"(]/m, `${file} imports`);
		}
	});
});
