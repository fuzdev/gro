import { describe, test, expect, vi, afterEach, onTestFinished } from 'vitest';
import { chmod, mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { install_with_cache_healing } from '#lib/npm_install_helpers.ts';
import { spawn } from '#lib/spawn.ts';
import { fs } from '#lib/fs.ts';
import { create_mock_process_output } from './process_output_mocks.ts';

/*

Runs the install through the real spawn module and a fake `npm` that reports only whether
a variable is set, by name, never a value, so no test output carries the environment.

*/

const NAMES = [
	'NODE_ENV',
	'npm_config_production',
	'npm_config_omit',
	'npm_config_registry',
	'npm_config_foo'
];

const report_names = NAMES.map(
	(name) => `[ "\${${name}+x}" = x ] && echo ${name}=set || echo ${name}=unset`
).join('\n');

/**
 * Writes a fake `npm` whose body runs with `$DIR` set, putting it first on `PATH`.
 */
const create_fake_npm = async (body: string): Promise<string> => {
	const dir = await mkdtemp(join(tmpdir(), 'gro_fake_npm_'));
	onTestFinished(() => rm(dir, { recursive: true, force: true }));
	const bin = join(dir, 'bin');
	await mkdir(bin);
	const npm = join(bin, 'npm');
	await writeFile(npm, `#!/bin/sh\nDIR=${dir}\n${body}\n`);
	await chmod(npm, 0o755);
	vi.stubEnv('PATH', `${bin}:${process.env.PATH ?? ''}`);
	return dir;
};

afterEach(() => {
	vi.unstubAllEnvs();
});

describe('install_with_cache_healing, through the spawn module', () => {
	test('prunes exactly the pruning vars, other npm_config_ vars reach the child', async () => {
		const dir = await create_fake_npm(`{\n${report_names}\n} > "$DIR/report"`);
		vi.stubEnv('NODE_ENV', 'production');
		vi.stubEnv('npm_config_production', 'true');
		vi.stubEnv('npm_config_omit', 'dev');
		vi.stubEnv('npm_config_registry', 'https://registry.example/');
		vi.stubEnv('npm_config_foo', 'bar');

		const result = await install_with_cache_healing(spawn, 'npm', {
			stderr: create_mock_process_output()
		});

		expect(result.ok).toBe(true);
		const report = (await readFile(join(dir, 'report'), 'utf8')).trim().split('\n');
		expect(report).toEqual([
			'NODE_ENV=unset',
			'npm_config_production=unset',
			'npm_config_omit=unset',
			'npm_config_registry=set',
			'npm_config_foo=set'
		]);
	});

	test('an unset LC_ALL stays unset for the child, a set one stays set', async () => {
		const dir = await create_fake_npm(
			`[ "\${LC_ALL+x}" = x ] && echo set > "$DIR/lc" || echo unset > "$DIR/lc"`
		);
		vi.stubEnv('LC_ALL', undefined);
		await install_with_cache_healing(spawn, 'npm', { stderr: create_mock_process_output() });
		expect((await readFile(join(dir, 'lc'), 'utf8')).trim()).toBe('unset');
		vi.stubEnv('LC_ALL', 'C');
		await install_with_cache_healing(spawn, 'npm', { stderr: create_mock_process_output() });
		expect((await readFile(join(dir, 'lc'), 'utf8')).trim()).toBe('set');
	});

	test('a Node lifecycle script sees LC_ALL unset', async () => {
		const dir = await create_fake_npm(
			`"${process.execPath}" -p "'LC_ALL' in process.env" > "$DIR/lc"`
		);
		vi.stubEnv('LC_ALL', undefined);
		await install_with_cache_healing(spawn, 'npm', { stderr: create_mock_process_output() });
		expect((await readFile(join(dir, 'lc'), 'utf8')).trim()).toBe('false');
	});

	test('tees stderr live while capturing it, and heals ETARGET across real stderr', async () => {
		const dir = await create_fake_npm(`
echo "$*" >> "$DIR/calls"
case "$1" in
cache) exit 0 ;;
esac
if [ ! -e "$DIR/healed" ]; then
	touch "$DIR/healed"
	echo 'npm error code ETARGET' 1>&2
	exit 1
fi
exit 0`);
		const stderr = create_mock_process_output();

		const result = await install_with_cache_healing(spawn, 'npm', { stderr });

		expect(result.ok).toBe(true);
		const teed = stderr.written.map((chunk) => new TextDecoder().decode(chunk as Uint8Array));
		expect(teed.join('')).toContain('ETARGET');
		expect((await readFile(join(dir, 'calls'), 'utf8')).trim().split('\n')).toEqual([
			'install',
			'cache clean --force',
			'install'
		]);
	});

	test('a failure with no ETARGET surfaces without healing, naming how it ended', async () => {
		const dir = await create_fake_npm(`echo called >> "$DIR/calls"\nexit 3`);
		const result = await install_with_cache_healing(spawn, 'npm', {
			stderr: create_mock_process_output()
		});
		expect(result).toEqual({ ok: false, stderr: '', detail: 'code 3' });
		expect((await readFile(join(dir, 'calls'), 'utf8')).trim().split('\n')).toEqual(['called']);
	});

	test('runs in the cwd handle', async () => {
		const dir = await create_fake_npm(`pwd -P > "$DIR/cwd"`);
		const work = join(dir, 'work');
		await mkdir(work);
		const handle = await fs.open_dir(work);
		await install_with_cache_healing(spawn, 'npm', {
			stderr: create_mock_process_output(),
			cwd: handle
		});
		expect((await readFile(join(dir, 'cwd'), 'utf8')).trim()).toBe(handle.path);
	});

	test('names a missing program in a failed result', async () => {
		vi.stubEnv('PATH', '/nonexistent');
		const result = await install_with_cache_healing(spawn, 'npm', {
			stderr: create_mock_process_output()
		});
		expect(result.ok).toBe(false);
		expect(result.detail).toContain('not_found');
	});
});
