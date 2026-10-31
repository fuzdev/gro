import { test, expect, assert } from 'vitest';
import * as esbuild from 'esbuild';
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { esbuild_plugin_sveltekit_local_imports } from '#lib/esbuild_plugin_sveltekit_local_imports.ts';

const IMPORTS_DEFAULT = { '#lib/*': './src/lib/*' };

const build_subpath_import = async (
	specifier: string,
	imports: Record<string, string> = IMPORTS_DEFAULT
) => {
	const dir = await mkdtemp(join(tmpdir(), 'gro_local_imports_'));
	try {
		await writeFile(join(dir, 'package.json'), JSON.stringify({ type: 'module', imports }));
		await mkdir(join(dir, 'src/lib'), { recursive: true });
		await writeFile(join(dir, 'src/lib/y.ts'), 'export const y: number = 1;\n');
		await writeFile(
			join(dir, 'src/lib/main.ts'),
			`import {y} from '${specifier}';\nconsole.log(y);\n`
		);
		return await esbuild
			.build({
				entryPoints: [join(dir, 'src/lib/main.ts')],
				plugins: [esbuild_plugin_sveltekit_local_imports()],
				format: 'esm',
				platform: 'node',
				bundle: true,
				write: false,
				logLevel: 'silent'
			})
			.catch((error: unknown) => error as esbuild.BuildFailure);
	} finally {
		await rm(dir, { recursive: true, force: true });
	}
};

const assert_builds = (result: Awaited<ReturnType<typeof build_subpath_import>>) => {
	assert(!(result instanceof Error), result instanceof Error ? result.message : undefined);
	expect(result.errors).toEqual([]);
	expect(result.outputFiles[0]!.text).toContain('var y = 1;');
};

test('a subpath import naming the real file builds', async () => {
	assert_builds(await build_subpath_import('#lib/y.ts'));
});

test("esbuild's `.js` to `.ts` rewrite of a subpath import is refused with the fix, as in the loader", async () => {
	const result = await build_subpath_import('#lib/y.js');
	assert(result instanceof Error);
	expect(result.errors).toHaveLength(1);
	expect(result.errors[0]!.text).toMatch(
		/^cannot resolve '#lib\/y\.js' from .*\/src\/lib\/main\.ts: no such file, but '#lib\/y\.ts' exists; write the real extension \(\.js never maps to \.ts\)$/
	);
});

test('a mapping that targets another extension on purpose builds, as in Node', async () => {
	assert_builds(await build_subpath_import('#x.js', { '#x.js': './src/lib/y.ts' }));
	assert_builds(await build_subpath_import('#w/y.js', { '#w/*.js': './src/lib/*.ts' }));
});

test('a subpath import naming no file fails as esbuild reports it', async () => {
	const result = await build_subpath_import('#lib/missing.ts');
	assert(result instanceof Error);
	expect(result.errors).toHaveLength(1);
	expect(result.errors[0]!.text).toContain('#lib/missing.ts');
});
