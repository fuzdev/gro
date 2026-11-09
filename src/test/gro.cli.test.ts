import { describe, test, expect, onTestFinished } from 'vitest';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, rmSync, symlinkSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { paths } from '#lib/paths.ts';

/*

Runs the CLI entry itself, under a given `PATH`, to see what a failure to find `node` prints.
The environment holds only `PATH`, so nothing from the host reaches the output.

*/

const run_gro = (path: string, cwd: string): { status: number | null; stderr: string } =>
	spawnSync(
		process.execPath,
		['--experimental-import-meta-resolve', join(paths.lib, 'gro.ts'), 'task'],
		{ cwd, env: { PATH: path }, encoding: 'utf8' }
	);

const create_dir = (): string => {
	const dir = mkdtempSync(join(tmpdir(), 'gro_cli_'));
	onTestFinished(() => rmSync(dir, { recursive: true, force: true }));
	return dir;
};

describe('the gro entry with no usable node', () => {
	test('no `node` on PATH prints the message without a stack and exits 1', () => {
		const { status, stderr } = run_gro('/nonexistent', create_dir());
		expect(status).toBe(1);
		expect(stderr).toContain('not_found');
		expect(stderr).toContain('absolute PATH entry');
		expect(stderr).not.toMatch(/^\s+at /m);
	});

	test('a `node` only on a relative PATH entry is refused the same way', () => {
		const dir = create_dir();
		mkdirSync(join(dir, 'bin'));
		symlinkSync(process.execPath, join(dir, 'bin', 'node'));
		const { status, stderr } = run_gro('bin', dir);
		expect(status).toBe(1);
		expect(stderr).toContain('not_found');
		expect(stderr).not.toMatch(/^\s+at /m);
	});
});
