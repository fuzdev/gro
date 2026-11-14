import { assert } from 'vitest';
import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

import { paths } from '#lib/paths.ts';

/*

What the tests of the runtime's builtin modules share: the rows of the builtin table, read from
its source, never listed, and each row's implementation, the script at
`crates/gro_runtime/js/builtin/<specifier>`.

*/

/** The directory of the builtins' implementations, their two declaration files, and tsconfig. */
export const BUILTIN_DIR = join(paths.root, 'crates/gro_runtime/js/builtin');

const TABLE_PATH = join(paths.root, 'crates/gro_runtime/src/resolver/builtin.rs');

const SPECIFIER_PREFIX = '@fuzdev/gro/';

export interface BuiltinRow {
	/** The builtin's specifier, like `@fuzdev/gro/args.ts`. */
	specifier: string;
	/** The host module's name, like `args`. */
	name: string;
	/** The implementation's path, the specifier under `BUILTIN_DIR`. */
	path: string;
}

/**
 * The builtin table's rows, in its order, read from its source.
 */
export const read_builtin_rows = (): Array<BuiltinRow> => {
	const source = readFileSync(TABLE_PATH, 'utf8');
	const rows = /macro_rules! builtin_rows \{([\s\S]*?)\n\}\n/.exec(source);
	assert.ok(rows, 'the table has its rows');
	return Array.from(rows[1]!.matchAll(/\$row!\(\s*"([^"]+)"/g), ([, specifier]) => {
		assert.ok(specifier!.startsWith(SPECIFIER_PREFIX) && specifier!.endsWith('.ts'), specifier);
		return {
			specifier: specifier!,
			name: specifier!.slice(SPECIFIER_PREFIX.length, -'.ts'.length),
			path: join(BUILTIN_DIR, specifier!)
		};
	});
};

/**
 * The implementations on disk, each as the specifier it's the file of, sorted.
 */
export const read_implementation_specifiers = (): Array<string> =>
	readdirSync(join(BUILTIN_DIR, SPECIFIER_PREFIX))
		.map((file) => SPECIFIER_PREFIX + file)
		.sort();

/**
 * A row's implementation, its source.
 */
export const read_implementation = (row: BuiltinRow): string => readFileSync(row.path, 'utf8');
