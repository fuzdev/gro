import { describe, test, expect, vi, afterEach } from 'vitest';

import { gro_plugin_sveltekit_library } from '#lib/gro_plugin_sveltekit_library.ts';
import { rewrite_dist_imports } from '#lib/dist_rewrite_imports.ts';
import type { PluginContext } from '#lib/plugin.ts';
import { create_mock_task_context } from './test_helpers.ts';

vi.mock('#lib/sveltekit_helpers.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/sveltekit_helpers.ts')>()),
	run_svelte_package: vi.fn(async () => undefined)
}));

vi.mock('#lib/dist_rewrite_imports.ts', () => ({
	rewrite_dist_imports: vi.fn(async () => undefined)
}));

vi.mock('#lib/package_json.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/package_json.ts')>()),
	package_json_load: vi.fn(async () => ({ name: 'a', version: '0' }))
}));

const original_argv = process.argv;

afterEach(() => {
	process.argv = original_argv;
	vi.clearAllMocks();
});

// the directory the plugin rewrites imports in after `svelte-package`
const to_rewritten_dir = async (
	forwarded: Array<string> | null,
	output?: string
): Promise<string> => {
	process.argv = [
		'node',
		'gro',
		'build',
		...(forwarded ? ['--', 'svelte-package', ...forwarded] : [])
	];
	const ctx = {
		...create_mock_task_context(),
		dev: false,
		watch: false
	} as unknown as PluginContext;
	await gro_plugin_sveltekit_library({
		svelte_package_options: output === undefined ? undefined : { output }
	}).setup!(ctx);
	expect(vi.mocked(rewrite_dist_imports)).toHaveBeenCalledTimes(1);
	return vi.mocked(rewrite_dist_imports).mock.calls[0]![0]!;
};

describe('gro_plugin_sveltekit_library forwarded output', () => {
	test('a forwarded `--output` or `-o` wins over the config', async () => {
		expect(await to_rewritten_dir(['--output', 'x'], 'configured')).toBe('x');
		vi.clearAllMocks();
		expect(await to_rewritten_dir(['--output=y', '-oz'], 'configured')).toBe('z');
	});

	test('falls back to the config, then `dist`', async () => {
		expect(await to_rewritten_dir(['--watch'], 'configured')).toBe('configured');
		vi.clearAllMocks();
		expect(await to_rewritten_dir(null)).toBe('dist');
	});
});
