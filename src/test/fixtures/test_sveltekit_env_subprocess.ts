// Test script to verify SvelteKit $env import shimming
// This runs in a separate Node.js process with the loader active

/* eslint-disable */

import { resolve } from 'node:path';

import { init_test_env } from '../test_helpers.ts';

async function runTest() {
	console.log('Testing SvelteKit $env import shimming...\n');

	// Set up the test environment first - this must happen before any dynamic imports
	await init_test_env();

	const VALUE = 'SOME_PUBLIC_ENV_VAR';

	try {
		// Test 1: Static imports via test fixture
		console.log('Testing static SvelteKit $env imports via fixture...');
		const fixture: any = await import(resolve('src/test/fixtures/test_sveltekit_env.ts'));

		if (fixture.exported_env_static_public !== VALUE) {
			console.error(
				`✗ Static import test failed: expected "${VALUE}", got "${
					fixture.exported_env_static_public
				}"`
			);
			process.exit(1);
		}
		console.log('✓ Static import test passed');

		// Test 2: Direct dynamic imports
		console.log('Testing dynamic SvelteKit $env imports...');
		const env_mod: any = await import('$env/static/public');

		if (!env_mod.PUBLIC_SOME_PUBLIC_ENV_VAR || env_mod.PUBLIC_SOME_PUBLIC_ENV_VAR !== VALUE) {
			console.error(
				`✗ Dynamic import test failed: expected "${VALUE}", got "${
					env_mod.PUBLIC_SOME_PUBLIC_ENV_VAR
				}"`
			);
			process.exit(1);
		}
		console.log('✓ Dynamic import test passed');

		console.log('\n✓ All SvelteKit $env shimming tests passed');
		process.exit(0);
	} catch (error) {
		console.error('✗ Test failed with error:', error);
		process.exit(1);
	}
}

runTest().catch((error) => {
	console.error('Test failed with error:', error);
	process.exit(1);
});
