import { describe, test, expect, afterEach } from 'vitest';

import { gro_plugin_sveltekit_app } from '#lib/gro_plugin_sveltekit_app.ts';
import { create_mock_plugin_context } from './test_helpers.ts';
import { create_mock_spawn } from './spawn_mocks.ts';

const spawn = create_mock_spawn();

afterEach(() => {
	spawn.reset();
});

const FORWARDED = ['--port', '0123', '--no-open', '--mode', '-foo', '--x='];

// the plugin's context, its forwarded args the CLI's after the first `--`
const to_ctx = (dev: boolean, raw_rest_args: ReadonlyArray<string>) =>
	create_mock_plugin_context({ dev, watch: true, host: { spawn }, raw_rest_args });

const setup = async (dev: boolean): Promise<void> => {
	await gro_plugin_sveltekit_app().setup!(to_ctx(dev, ['--', 'vite', ...FORWARDED]));
};

describe('gro_plugin_sveltekit_app forwarded args', () => {
	test('`vite dev` gets the text verbatim', async () => {
		await setup(true);
		const { calls } = spawn;
		expect(calls[0]!.method).toBe('start');
		expect(calls[0]!.args).toEqual(['dev', ...FORWARDED]);
		expect(calls[0]!.program_options).toEqual({ pass_env: 'all' });
	});

	test('`vite build` gets the text verbatim', async () => {
		await setup(false);
		const { calls } = spawn;
		expect(calls[0]!.method).toBe('run');
		expect(calls[0]!.args).toEqual(['build', ...FORWARDED]);
		expect(calls[0]!.program_options).toEqual({ pass_env: 'all' });
	});
});

test('a forwarded help flag is refused for `vite dev` and `vite build`, a version flag left to vite', async () => {
	for (const dev of [true, false]) {
		await expect(
			gro_plugin_sveltekit_app().setup!(to_ctx(dev, ['--', 'vite', '--help']))
		).rejects.toThrow('`--help` forwarded to vite asks it for its help');
	}
	expect(spawn.calls).toHaveLength(0);
	// Vite ignores a version flag after the command Gro gives it
	await gro_plugin_sveltekit_app().setup!(to_ctx(false, ['--', 'vite', '-v']));
	expect(spawn.calls[0]!.args).toEqual(['build', '-v']);
});
