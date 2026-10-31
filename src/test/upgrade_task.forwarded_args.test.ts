import { describe, test, expect, vi, afterEach } from 'vitest';
import { args_parse } from '@fuzdev/fuz_util/args.ts';

import { Args, task } from '#lib/upgrade.task.ts';
import { install_with_cache_healing_or_throw } from '#lib/npm_install_helpers.ts';
import { create_mock_task_context } from './test_helpers.ts';

vi.mock('#lib/npm_install_helpers.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/npm_install_helpers.ts')>()),
	install_with_cache_healing_or_throw: vi.fn(async () => undefined)
}));

vi.mock('#lib/cli.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/cli.ts')>()),
	spawn_cli: vi.fn(async () => ({ ok: true }))
}));

vi.mock('#lib/package_json.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/package_json.ts')>()),
	package_json_load: vi.fn(async () => ({
		name: 'a',
		version: '0',
		dependencies: { b: '^1.0.0' }
	}))
}));

const original_argv = process.argv;

afterEach(() => {
	process.argv = original_argv;
	vi.clearAllMocks();
});

describe('upgrade task forwarded args', () => {
	test('the package manager gets the text verbatim after the install args', async () => {
		const forwarded = ['--no-audit', '--fetch-retries', '01', '--x='];
		process.argv = ['node', 'gro', 'upgrade', '--', 'npm', ...forwarded];
		const parsed = args_parse({ 'no-pull': true, pull: false, force: true }, Args);
		expect(parsed.ok).toBe(true);
		await task.run(create_mock_task_context(parsed.ok ? parsed.value : {}));
		expect(vi.mocked(install_with_cache_healing_or_throw)).toHaveBeenCalledTimes(1);
		const [pm_cli, options] = vi.mocked(install_with_cache_healing_or_throw).mock.calls[0]!;
		expect(pm_cli).toBe('npm');
		expect(options?.install_args).toEqual(['b@latest', '--force', ...forwarded]);
	});
});
