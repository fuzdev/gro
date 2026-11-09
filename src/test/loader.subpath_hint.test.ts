import { test, expect } from 'vitest';
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';

import { spawn_with_loader_args } from '#lib/gro_helpers.ts';

import { LOADER_SOURCE_PATH, TEST_TIMEOUT_MD } from './test_helpers.ts';

test(
	"a subpath import's sibling hint names a specifier only when `imports` maps it to the sibling",
	async () => {
		const dir = await mkdtemp(join(tmpdir(), 'gro_loader_subpath_hint_'));
		try {
			await writeFile(
				join(dir, 'package.json'),
				JSON.stringify({
					type: 'module',
					imports: { '#lib/*': './src/lib/*', '#cfg.js': './src/lib/config.js' }
				})
			);
			await mkdir(join(dir, 'src/lib'), { recursive: true });
			await writeFile(join(dir, 'src/lib/y.ts'), 'export const y = 1;\n');
			await writeFile(join(dir, 'src/lib/config.ts'), 'export const c = 1;\n');
			await writeFile(
				join(dir, 'main.ts'),
				`const messages: Array<string> = [];
for (const specifier of ['#lib/y.js', '#cfg.js', '#lib/y.js?raw']) {
	try {
		await import(specifier);
		messages.push('loaded');
	} catch (error) {
		messages.push((error as Error).message);
	}
}
console.log(JSON.stringify(messages));
`
			);
			const { status, stdout } = spawnSync(
				process.execPath,
				spawn_with_loader_args(LOADER_SOURCE_PATH, join(dir, 'main.ts'), [], undefined),
				{ cwd: dir, encoding: 'utf8' }
			);
			expect(status).toBe(0);
			const [lib_message, cfg_message, raw_message] = JSON.parse(stdout) as Array<string>;
			expect(lib_message).toContain(
				"; '#lib/y.ts' exists; write the real extension (.js never maps to .ts)"
			);
			// the hint keeps `?raw`
			expect(raw_message).toContain(
				"; '#lib/y.ts?raw' exists; write the real extension (.js never maps to .ts)"
			);
			// `#cfg.ts` isn't mapped, so the hint names the file
			expect(cfg_message).not.toContain("'#cfg.ts'");
			expect(cfg_message).toContain(
				`; the file ${join(dir, 'src/lib/config.ts')} exists, but '#cfg.js' maps to ` +
					`${join(dir, 'src/lib/config.js')} (.js never maps to .ts)`
			);
		} finally {
			await rm(dir, { recursive: true, force: true });
		}
	},
	TEST_TIMEOUT_MD
);
