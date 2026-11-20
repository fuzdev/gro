import { test, assert, vi } from 'vitest';

import { GRO_BUILTIN_TASK_GRANTS } from '#lib/gro_builtin_tasks.ts';
import { create_builtin_host, type BuiltinHostRoot } from '#lib/builtin_host.ts';
import { env } from '#lib/env.ts';
import type { Spawn } from '#lib/spawn.ts';
import type { ProcessOutput } from '#lib/stdout.ts';

import { create_mock_fs_paths } from './fs_mocks.ts';

// every object and array the table holds, the table first
const to_values = (value: unknown, values: Array<object> = []): Array<object> => {
	if (value === null || typeof value !== 'object') return values;
	values.push(value);
	for (const child of Object.values(value)) to_values(child, values);
	return values;
};

test('the grants table is deep-frozen, its objects null-prototype', () => {
	const values = to_values(GRO_BUILTIN_TASK_GRANTS);
	assert.ok(values.some((value) => Array.isArray(value)));
	for (const value of values) {
		assert.ok(Object.isFrozen(value));
		if (!Array.isArray(value)) assert.strictEqual(Object.getPrototypeOf(value), null);
	}
});

test("a property added to `Object.prototype` widens no builtin's host", () => {
	const root: BuiltinHostRoot = {
		env,
		spawn: { program: vi.fn(), find: vi.fn() } as unknown as Spawn,
		stdout: {} as ProcessOutput,
		stderr: {} as ProcessOutput
	};
	const invocation = { fs_paths: create_mock_fs_paths(), import_module: vi.fn() };
	const polluted = Object.prototype as unknown as Record<string, unknown>;
	const names = ['fs', 'env', 'programs', 'stdout', 'stderr', 'project_modules', 'invokes'];
	for (const name of names)
		polluted[name] = name === 'programs' ? { sh_cli: { name: 'sh' } } : true;
	try {
		const { format } = GRO_BUILTIN_TASK_GRANTS;
		assert.ok(format);
		assert.deepEqual(Object.keys(create_builtin_host(root, format, invocation)), ['fs_paths']);
		assert.strictEqual(GRO_BUILTIN_TASK_GRANTS.check?.programs?.sh_cli, undefined);
	} finally {
		for (const name of names) delete polluted[name];
	}
});
