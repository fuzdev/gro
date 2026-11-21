import { test, assert } from 'vitest';
import {
	get_schema_entries,
	get_schema_meta,
	unwrap_schema
} from '@fuzdev/fuz_schema/introspect.ts';
import * as s from '@fuzdev/fuz_schema/schema.ts';

import { GRO_BUILTIN_TASK_GRANTS } from '#lib/gro_builtin_tasks.ts';
import { GRO_BUILTIN_TASK_IMPORTS } from '#lib/gro_builtin_task_imports.ts';
import { task as check_task, Args as CheckArgs } from '#lib/check.task.ts';

test('each invoke names a builtin, and its `when` a boolean arg of the invoking task, off by default', async () => {
	for (const [task_name, grants] of Object.entries(GRO_BUILTIN_TASK_GRANTS)) {
		if (!grants?.invokes) continue;
		for (const [invoked, invoke] of Object.entries(grants.invokes)) {
			const label = `${task_name}'s invoke of \`${invoked}\``;
			assert.ok(Object.hasOwn(GRO_BUILTIN_TASK_GRANTS, invoked), label);
			if (invoke.when === undefined) continue;
			const { task } =
				await GRO_BUILTIN_TASK_IMPORTS[task_name as keyof typeof GRO_BUILTIN_TASK_IMPORTS]();
			assert.ok(task.Args, label);
			const entries = get_schema_entries(task.Args);
			assert.ok(entries && Object.hasOwn(entries, invoke.when), label);
			const parsed = s.parse(task.Args, {});
			assert(parsed.ok, label);
			// off by default, so a runtime holds the invoke only when the command line turns it on
			assert.strictEqual(parsed.value[invoke.when], false, label);
		}
	}
});

test("`check`'s closure holds `build` only under `--build`, which it defaults off", () => {
	assert.deepEqual(GRO_BUILTIN_TASK_GRANTS.check?.invokes?.build, { when: 'build' });
	assert.deepEqual(check_task.grants.invokes.build, { when: 'build' });
	const parsed = s.parse(CheckArgs, {});
	assert(parsed.ok);
	assert.strictEqual(parsed.value.build, false);
});

test('the args a native closure reads by spelling are booleans with no alias, off by default', async () => {
	// the binary reads a command's `when` args, `install`, and a check-capable command's `check`
	// from the command line by their spelling (`--build`, `--no-build`), never through `Args`,
	// so an alias on either side of a pair, or a non-boolean arg, would make it disagree with Node
	// whether `name`'s closure can hold `npm_cli`, through any invoke, where the binary reads
	// `install` from the command line and its default isn't read
	const reaches_npm = (name: string, seen: Set<string> = new Set()): boolean => {
		const grants = GRO_BUILTIN_TASK_GRANTS[name as keyof typeof GRO_BUILTIN_TASK_GRANTS];
		if (!grants || seen.has(name)) return false;
		seen.add(name);
		return (
			Object.hasOwn(grants.programs ?? {}, 'npm_cli') ||
			Object.keys(grants.invokes ?? {}).some((invoked) => reaches_npm(invoked, seen))
		);
	};
	const check_capable: Set<string> = new Set();
	for (const grants of Object.values(GRO_BUILTIN_TASK_GRANTS)) {
		for (const [invoked, invoke] of Object.entries(grants?.invokes ?? {})) {
			if (invoke.check) check_capable.add(invoked);
		}
	}
	let checked = 0;
	for (const [task_name, grants] of Object.entries(GRO_BUILTIN_TASK_GRANTS)) {
		// a builtin with no grants runs on the Node Gro, its args never read natively
		if (!grants) continue;
		const read_by_spelling: Set<string> = new Set(reaches_npm(task_name) ? ['install'] : []);
		for (const invoke of Object.values(grants.invokes ?? {})) {
			if (invoke.when !== undefined) read_by_spelling.add(invoke.when);
		}
		if (check_capable.has(task_name)) read_by_spelling.add('check');
		const { task } =
			await GRO_BUILTIN_TASK_IMPORTS[task_name as keyof typeof GRO_BUILTIN_TASK_IMPORTS]();
		const entries = task.Args ? get_schema_entries(task.Args) : null;
		const defaults = task.Args ? s.parse(task.Args, {}) : null;
		for (const arg of read_by_spelling) {
			const label = `${task_name}'s \`${arg}\``;
			if (!entries || !Object.hasOwn(entries, arg)) {
				// a check-capable builtin has the `check` its invokers pass
				assert.notStrictEqual(arg, 'check', label);
				continue;
			}
			assert.strictEqual(unwrap_schema(entries[arg]!).kind, 'boolean', label);
			assert.strictEqual(get_schema_meta(entries[arg]!, 'aliases'), undefined, label);
			const negated = `no-${arg}`;
			if (Object.hasOwn(entries, negated)) {
				assert.strictEqual(get_schema_meta(entries[negated]!, 'aliases'), undefined, label);
			}
			assert(defaults?.ok, label);
			assert.strictEqual(defaults.value[arg], false, label);
			checked++;
		}
	}
	// `check`'s `build` and `install`, `sync`'s `install`, and `gen`'s and `format`'s `check`
	assert.ok(checked >= 5, `${checked}`);
});
