import { test, assert, onTestFinished } from 'vitest';
import { mkdirSync, mkdtempSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';

import {
	npm_install_check,
	type NpmInstallCheck,
	type NpmInstallMismatchKind
} from '#lib/npm_install_check.ts';
import { LOCKFILE_FILENAME, NPM_HIDDEN_LOCKFILE_PATH } from '#lib/constants.ts';
import { is_task_error } from '#lib/task_error.ts';
import { is_fs_error } from '#lib/fs_error.ts';

import { create_project_fs_paths } from './fs_test_helpers.ts';

interface Entry {
	version?: string;
	resolved?: string;
	integrity?: string;
	link?: boolean;
	optional?: boolean;
	os?: Array<string>;
	dev?: boolean;
}

// a locked package as npm writes it, with distinct values so a swapped field can't match
const to_entry = (name: string, version: string, fields: Partial<Entry> = {}): Entry => ({
	version,
	resolved: `https://registry.example/${name}/-/${name}-${version}.tgz`,
	integrity: `sha512-${name}-${version}`,
	...fields
});

interface Project {
	package_json?: object;
	lockfile?: object | string | null;
	hidden?: object | null;
	// the installed packages' versions by lockfile key, `null` for a package with no `package.json`
	installed?: Record<string, string | null>;
}

const A = 'node_modules/a';
const B = 'node_modules/@s/b';

// a project whose install matches its lockfile: `a` and `@s/b`, a dev dependency
const MATCHING = (): Required<Omit<Project, 'lockfile' | 'hidden'>> & {
	lockfile: { lockfileVersion: number; packages: Record<string, object> };
	hidden: { lockfileVersion: number; packages: Record<string, object> };
} => {
	const package_json = {
		name: 'p',
		dependencies: { a: '^1.0.0' },
		devDependencies: { '@s/b': '~2.0.0' }
	};
	const packages: Record<string, object> = {
		[A]: to_entry('a', '1.2.3'),
		[B]: to_entry('b', '2.0.1', { dev: true })
	};
	return {
		package_json,
		lockfile: {
			lockfileVersion: 3,
			packages: {
				'': { name: 'p', dependencies: { a: '^1.0.0' }, devDependencies: { '@s/b': '~2.0.0' } },
				...packages
			}
		},
		hidden: { lockfileVersion: 3, packages: { ...packages } },
		installed: { [A]: '1.2.3', [B]: '2.0.1' }
	};
};

const check = async (project: Project): Promise<NpmInstallCheck> => {
	const dir = mkdtempSync(join(tmpdir(), 'gro_npm_install_check_'));
	onTestFinished(() => rmSync(dir, { recursive: true }));
	const write = (path: string, contents: string): void => {
		mkdirSync(dirname(join(dir, path)), { recursive: true });
		writeFileSync(join(dir, path), contents);
	};
	if (project.package_json) write('package.json', JSON.stringify(project.package_json));
	if (project.lockfile != null) {
		write(
			LOCKFILE_FILENAME,
			typeof project.lockfile === 'string' ? project.lockfile : JSON.stringify(project.lockfile)
		);
	}
	if (project.hidden != null) write(NPM_HIDDEN_LOCKFILE_PATH, JSON.stringify(project.hidden));
	for (const [key, version] of Object.entries(project.installed ?? {})) {
		if (version === null) mkdirSync(join(dir, key), { recursive: true });
		else write(key + '/package.json', JSON.stringify({ name: key, version }));
	}
	return npm_install_check(await create_project_fs_paths(dir));
};

const to_kinds = (checked: NpmInstallCheck): Array<[NpmInstallMismatchKind, string]> => {
	assert(checked.checked);
	return checked.mismatches.map((m) => [m.kind, m.key]);
};

test('an install that matches its lockfile has no mismatches', async () => {
	const checked = await check(MATCHING());
	assert(checked.checked);
	assert.deepEqual(checked.mismatches, []);
	assert.deepEqual(checked.linked, []);
});

test('with no package-lock.json, or one with no `packages`, the install is not checked', async () => {
	const none = await check({ ...MATCHING(), lockfile: null });
	assert(!none.checked);
	assert.strictEqual(none.reason, 'the project has no package-lock.json');
	const v1 = await check({ ...MATCHING(), lockfile: { lockfileVersion: 1, dependencies: {} } });
	assert(!v1.checked);
	assert.include(v1.reason, 'has no `packages`');
});

test('a `package.json` spec the lockfile records otherwise is a `spec` mismatch', async () => {
	const project = MATCHING();
	project.package_json = { ...project.package_json, dependencies: { a: '^1.3.0', c: '1' } };
	const checked = await check(project);
	assert.deepEqual(to_kinds(checked), [
		['spec', 'dependencies.a'],
		['spec', 'dependencies.c']
	]);
	assert(checked.checked);
	assert.strictEqual(
		checked.mismatches[0]!.message,
		`package.json's dependencies has a "^1.3.0", package-lock.json's "^1.0.0"`
	);
});

test('no hidden lockfile is a `no_hidden_lockfile` mismatch, the installed versions still read', async () => {
	const project = MATCHING();
	project.installed[A] = '1.2.4';
	assert.deepEqual(to_kinds(await check({ ...project, hidden: null })), [
		['no_hidden_lockfile', NPM_HIDDEN_LOCKFILE_PATH],
		['version', A]
	]);
});

test('a locked package the hidden lockfile lacks is `unrecorded`, but for an optional or platform one', async () => {
	const project = MATCHING();
	delete project.hidden.packages[A];
	assert.deepEqual(to_kinds(await check(project)), [['unrecorded', A]]);
	for (const fields of [{ optional: true }, { os: ['other'] }]) {
		const skippable = MATCHING();
		const key = 'node_modules/opt';
		skippable.lockfile.packages[key] = to_entry('opt', '1.0.0', fields);
		assert.deepEqual(to_kinds(await check(skippable)), [], JSON.stringify(fields));
	}
});

test('a locked field the hidden lockfile records otherwise is a `differs` mismatch', async () => {
	for (const field of ['version', 'resolved', 'integrity', 'link'] as const) {
		const project = MATCHING();
		project.hidden.packages[A] = { ...project.hidden.packages[A], [field]: 'other' };
		const checked = await check(project);
		const kinds = to_kinds(checked);
		assert.deepEqual(kinds[0], ['differs', A], field);
		assert(checked.checked);
		assert.include(checked.mismatches[0]!.message, `${A}'s ${field} is`, field);
	}
});

test('a package the hidden lockfile records and the lockfile does not is `extraneous`', async () => {
	const project = MATCHING();
	project.hidden.packages['node_modules/x'] = to_entry('x', '1.0.0');
	assert.deepEqual(to_kinds(await check(project)), [['extraneous', 'node_modules/x']]);
});

test('a locked package with no installed `package.json` is `not_installed`', async () => {
	const project = MATCHING();
	project.installed[B] = null;
	assert.deepEqual(to_kinds(await check(project)), [['not_installed', B]]);
});

test('an installed version the lockfile locks otherwise is a `version` mismatch', async () => {
	const project = MATCHING();
	project.installed[A] = '1.2.4';
	const checked = await check(project);
	assert.deepEqual(to_kinds(checked), [['version', A]]);
	assert(checked.checked);
	assert.strictEqual(checked.mismatches[0]!.message, `${A} is "1.2.4" installed, "1.2.3" locked`);
});

test('a package installed as a symlink is skipped and named as linked', async () => {
	const dir = mkdtempSync(join(tmpdir(), 'gro_npm_install_check_linked_'));
	onTestFinished(() => rmSync(dir, { recursive: true }));
	const project = MATCHING();
	writeFileSync(join(dir, 'package.json'), JSON.stringify(project.package_json));
	writeFileSync(join(dir, LOCKFILE_FILENAME), JSON.stringify(project.lockfile));
	mkdirSync(join(dir, 'node_modules/@s/b'), { recursive: true });
	writeFileSync(join(dir, 'node_modules/@s/b/package.json'), '{"version": "2.0.1"}');
	// `a` linked to a sibling a version ahead, missing from the hidden lockfile too
	mkdirSync(join(dir, 'sibling'));
	writeFileSync(join(dir, 'sibling/package.json'), '{"version": "9.9.9"}');
	symlinkSync('../sibling', join(dir, A));
	const hidden = { ...project.hidden, packages: { [B]: project.hidden.packages[B] } };
	writeFileSync(join(dir, NPM_HIDDEN_LOCKFILE_PATH), JSON.stringify(hidden));
	const checked = await npm_install_check(await create_project_fs_paths(dir));
	assert(checked.checked);
	assert.deepEqual(checked.mismatches, []);
	assert.deepEqual(checked.linked, [A]);
});

test('a lockfile that is not JSON fails naming it', async () => {
	let error: unknown;
	try {
		await check({ ...MATCHING(), lockfile: '{' });
	} catch (e) {
		error = e;
	}
	assert(is_task_error(error));
	assert.include(error.message, "package-lock.json isn't valid JSON");
});

test('a `devOptional` locked package may be absent from the hidden lockfile and `node_modules`', async () => {
	const project = MATCHING();
	const key = 'node_modules/dev_opt';
	project.lockfile.packages[key] = to_entry('dev_opt', '1.0.0', { devOptional: true } as never);
	assert.deepEqual(to_kinds(await check(project)), []);
});

test('an `optionalDependencies` spec the lockfile records otherwise is a `spec` mismatch', async () => {
	const project = MATCHING();
	project.package_json = { ...project.package_json, optionalDependencies: { o: '^1.0.0' } };
	assert.deepEqual(to_kinds(await check(project)), [['spec', 'optionalDependencies.o']]);
});

test('a locked `link` entry is never read for an installed version, symlinked or not', async () => {
	const project = MATCHING();
	const key = 'node_modules/w';
	// a workspace member or `file:` dependency, whose entry locks no version
	const entry = { resolved: 'packages/w', link: true };
	project.lockfile.packages[key] = entry;
	project.hidden.packages[key] = entry;
	project.installed[key] = '3.0.0';
	assert.deepEqual(to_kinds(await check(project)), []);
});

test('the hidden lockfile and lockfile keys beneath a linked package are skipped', async () => {
	const dir = mkdtempSync(join(tmpdir(), 'gro_npm_install_check_linked_nested_'));
	onTestFinished(() => rmSync(dir, { recursive: true }));
	const project = MATCHING();
	const nested = A + '/node_modules/n';
	project.lockfile.packages[nested] = to_entry('n', '1.0.0');
	writeFileSync(join(dir, 'package.json'), JSON.stringify(project.package_json));
	writeFileSync(join(dir, LOCKFILE_FILENAME), JSON.stringify(project.lockfile));
	mkdirSync(join(dir, 'node_modules/@s/b'), { recursive: true });
	writeFileSync(join(dir, 'node_modules/@s/b/package.json'), '{"version": "2.0.1"}');
	mkdirSync(join(dir, 'sibling'));
	writeFileSync(join(dir, 'sibling/package.json'), '{"version": "1.2.3"}');
	symlinkSync('../sibling', join(dir, A));
	const hidden = {
		...project.hidden,
		packages: {
			[B]: project.hidden.packages[B],
			[A + '/node_modules/only_installed']: to_entry('only_installed', '1.0.0')
		}
	};
	writeFileSync(join(dir, NPM_HIDDEN_LOCKFILE_PATH), JSON.stringify(hidden));
	const checked = await npm_install_check(await create_project_fs_paths(dir));
	assert(checked.checked);
	assert.deepEqual(checked.mismatches, []);
	assert.deepEqual(checked.linked, [A]);
});

test('an installed `package.json` with no version is a `version` mismatch saying so', async () => {
	const project = MATCHING();
	project.installed[A] = null;
	const checked = await check(project);
	// the directory's there, its `package.json` written below with no version
	assert.deepEqual(to_kinds(checked), [['not_installed', A]]);
	const dir = mkdtempSync(join(tmpdir(), 'gro_npm_install_check_no_version_'));
	onTestFinished(() => rmSync(dir, { recursive: true }));
	const matching = MATCHING();
	writeFileSync(join(dir, 'package.json'), JSON.stringify(matching.package_json));
	writeFileSync(join(dir, LOCKFILE_FILENAME), JSON.stringify(matching.lockfile));
	mkdirSync(join(dir, A), { recursive: true });
	mkdirSync(join(dir, B), { recursive: true });
	writeFileSync(join(dir, NPM_HIDDEN_LOCKFILE_PATH), JSON.stringify(matching.hidden));
	writeFileSync(join(dir, A, 'package.json'), '{"name": "a"}');
	writeFileSync(join(dir, B, 'package.json'), '{"version": "2.0.1"}');
	const no_version = await npm_install_check(await create_project_fs_paths(dir));
	assert(no_version.checked);
	assert.deepEqual(no_version.mismatches, [
		{ kind: 'version', key: A, message: `${A}'s package.json has no version, "1.2.3" locked` }
	]);
});

test('with nothing locked to install, a missing hidden lockfile is no mismatch', async () => {
	const none = MATCHING();
	none.package_json = { name: 'p' };
	none.lockfile.packages = { '': { name: 'p' } };
	assert.deepEqual(to_kinds(await check({ ...none, hidden: null, installed: {} })), []);
	// only an optional package locked, which npm may leave uninstalled
	const optional = MATCHING();
	optional.package_json = { name: 'p' };
	optional.lockfile.packages = {
		'': { name: 'p' },
		'node_modules/o': to_entry('o', '1.0.0', { optional: true })
	};
	assert.deepEqual(to_kinds(await check({ ...optional, hidden: null, installed: {} })), []);
});

test("a `node_modules` that's a symlink is not checked, the link not followed", async () => {
	const dir = mkdtempSync(join(tmpdir(), 'gro_npm_install_check_shared_'));
	onTestFinished(() => rmSync(dir, { recursive: true }));
	const project = MATCHING();
	mkdirSync(join(dir, 'p'));
	writeFileSync(join(dir, 'p/package.json'), JSON.stringify(project.package_json));
	writeFileSync(join(dir, 'p', LOCKFILE_FILENAME), JSON.stringify(project.lockfile));
	// another checkout's install, outside the project, a version ahead
	mkdirSync(join(dir, 'other/node_modules/a'), { recursive: true });
	writeFileSync(join(dir, 'other/node_modules/a/package.json'), '{"version": "9.9.9"}');
	symlinkSync('../other/node_modules', join(dir, 'p/node_modules'));
	const checked = await npm_install_check(await create_project_fs_paths(join(dir, 'p')));
	assert(!checked.checked);
	assert.strictEqual(checked.reason, 'node_modules is a symlink');
});

test('a package beneath a scope directory linked from outside the project is linked', async () => {
	const dir = mkdtempSync(join(tmpdir(), 'gro_npm_install_check_scope_'));
	onTestFinished(() => rmSync(dir, { recursive: true }));
	const project = MATCHING();
	mkdirSync(join(dir, 'p/node_modules/a'), { recursive: true });
	writeFileSync(join(dir, 'p/package.json'), JSON.stringify(project.package_json));
	writeFileSync(join(dir, 'p', LOCKFILE_FILENAME), JSON.stringify(project.lockfile));
	writeFileSync(join(dir, 'p/node_modules/a/package.json'), '{"version": "1.2.3"}');
	writeFileSync(join(dir, 'p', NPM_HIDDEN_LOCKFILE_PATH), JSON.stringify(project.hidden));
	// the `@s` scope from elsewhere, its `b` a version ahead
	mkdirSync(join(dir, 'other/@s/b'), { recursive: true });
	writeFileSync(join(dir, 'other/@s/b/package.json'), '{"version": "9.9.9"}');
	symlinkSync('../../other/@s', join(dir, 'p/node_modules/@s'));
	const checked = await npm_install_check(await create_project_fs_paths(join(dir, 'p')));
	assert(checked.checked);
	assert.deepEqual(checked.mismatches, []);
	assert.deepEqual(checked.linked, [B]);
});

test('a key whose stat fails other than by leaving the project fails the check, never counted linked', async () => {
	const dir = mkdtempSync(join(tmpdir(), 'gro_npm_install_check_unreadable_'));
	const project = MATCHING();
	writeFileSync(join(dir, 'package.json'), JSON.stringify(project.package_json));
	writeFileSync(join(dir, LOCKFILE_FILENAME), JSON.stringify(project.lockfile));
	mkdirSync(join(dir, 'node_modules/a'), { recursive: true });
	writeFileSync(join(dir, 'node_modules/a/package.json'), '{"version": "1.2.3"}');
	// `@s` a symlink to itself, so a path through it loops
	symlinkSync('@s', join(dir, 'node_modules/@s'));
	writeFileSync(join(dir, NPM_HIDDEN_LOCKFILE_PATH), JSON.stringify(project.hidden));
	onTestFinished(() => rmSync(dir, { recursive: true }));
	let error: unknown;
	try {
		await npm_install_check(await create_project_fs_paths(dir));
	} catch (e) {
		error = e;
	}
	assert(is_fs_error(error));
	assert.notStrictEqual(error.kind, 'outside_root');
});

test('an installed package with no version is no mismatch when the lockfile locks none', async () => {
	const dir = mkdtempSync(join(tmpdir(), 'gro_npm_install_check_versionless_'));
	onTestFinished(() => rmSync(dir, { recursive: true }));
	const project = MATCHING();
	// a versionless git dependency, its lock entry and hidden lockfile entry with no version
	const entry = { resolved: 'git+ssh://example/g.git#abc', integrity: 'sha512-g' };
	project.lockfile.packages['node_modules/g'] = entry;
	project.hidden.packages['node_modules/g'] = entry;
	writeFileSync(join(dir, 'package.json'), JSON.stringify(project.package_json));
	writeFileSync(join(dir, LOCKFILE_FILENAME), JSON.stringify(project.lockfile));
	for (const [key, version] of [
		[A, '1.2.3'],
		[B, '2.0.1']
	] as const) {
		mkdirSync(join(dir, key), { recursive: true });
		writeFileSync(join(dir, key, 'package.json'), JSON.stringify({ version }));
	}
	mkdirSync(join(dir, 'node_modules/g'), { recursive: true });
	writeFileSync(join(dir, 'node_modules/g/package.json'), '{"name": "g"}');
	writeFileSync(join(dir, NPM_HIDDEN_LOCKFILE_PATH), JSON.stringify(project.hidden));
	const checked = await npm_install_check(await create_project_fs_paths(dir));
	assert(checked.checked);
	assert.deepEqual(checked.mismatches, []);
});

test('an installed version the lockfile locks none of is a `version` mismatch', async () => {
	const project = MATCHING();
	const { version: _, ...unversioned } = project.lockfile.packages[A] as { version: string };
	project.lockfile.packages[A] = unversioned;
	project.hidden.packages[A] = unversioned;
	const checked = await check(project);
	assert.deepEqual(to_kinds(checked), [['version', A]]);
	assert(checked.checked);
	assert.strictEqual(checked.mismatches[0]!.message, `${A} is "1.2.3" installed, none locked`);
});
