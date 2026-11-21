import { describe, test, expect, vi, afterEach } from 'vitest';

import { gro_plugin_sveltekit_library } from '#lib/gro_plugin_sveltekit_library.ts';
import { rewrite_dist_imports } from '#lib/dist_rewrite_imports.ts';
import { create_mock_plugin_context } from './test_helpers.ts';
import { create_mock_spawn } from './spawn_mocks.ts';

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

afterEach(() => {
	vi.clearAllMocks();
});

// a build's plugin context, its forwarded args the CLI's after the first `--`,
// with no fs or spawn of its own, since what reaches them is mocked
const to_ctx = (raw_rest_args: ReadonlyArray<string>) =>
	create_mock_plugin_context({
		dev: false,
		watch: false,
		host: { fs_paths: {} as never, spawn: create_mock_spawn() },
		raw_rest_args
	});

// the directory the plugin rewrites imports in after `svelte-package`
const to_rewritten_dir = async (
	forwarded: Array<string> | null,
	output?: string
): Promise<string> => {
	await gro_plugin_sveltekit_library({
		svelte_package_options: output === undefined ? undefined : { output }
	}).setup!(to_ctx(forwarded ? ['--', 'svelte-package', ...forwarded] : []));
	expect(vi.mocked(rewrite_dist_imports)).toHaveBeenCalledTimes(1);
	return vi.mocked(rewrite_dist_imports).mock.calls[0]![1]!;
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

test('a forwarded help or version flag is refused before `svelte-package` runs', async () => {
	const { run_svelte_package } = vi.mocked(await import('#lib/sveltekit_helpers.ts'));
	for (const flag of ['-h', '--version']) {
		await expect(
			gro_plugin_sveltekit_library().setup!(to_ctx(['--', 'svelte-package', flag]))
		).rejects.toThrow(`\`${flag}\` forwarded to svelte-package asks it for its `);
	}
	expect(run_svelte_package).not.toHaveBeenCalled();
});
