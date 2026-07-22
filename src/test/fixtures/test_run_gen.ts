// Test script to verify run_gen basic behavior
// This runs in a separate Node.js process with the loader active

const run_test = async () => {
	// console.log('Testing run_gen basic behavior...\n');

	// Import what we need for the test
	const { resolve } = await import('node:path');
	const { Timings } = await import('@fuzdev/fuz_util/timings.ts');
	const { Logger } = await import('@fuzdev/fuz_util/log.ts');

	const { run_gen } = await import('$lib/run_gen.ts');
	const { load_gro_config } = await import('$lib/gro_config.ts');
	const { Filer } = await import('$lib/filer.ts');

	// console.log('✓ All imports work');

	const log = new Logger('test__run_gen');

	// Simple test with one module
	const path_id = resolve('src/test.gen.ts');
	let file_generated = false as boolean;

	const mod = {
		id: path_id,
		mod: {
			gen: () => {
				file_generated = true;
				return {
					filename: 'test_output.ts',
					content: 'export const test = "ok";'
				};
			}
		}
	};

	const gen_results = await run_gen(
		[mod],
		await load_gro_config(),
		new Filer(),
		log,
		new Timings(),
		Function.prototype as any
	);

	if (gen_results.input_count !== 1) {
		throw new Error(`Expected input_count 1, got ${gen_results.input_count}`);
	}
	if (gen_results.output_count !== 1) {
		throw new Error(`Expected output_count 1, got ${gen_results.output_count}`);
	}
	if (gen_results.successes.length !== 1) {
		throw new Error(`Expected 1 success, got ${gen_results.successes.length}`);
	}
	if (gen_results.failures.length !== 0) {
		throw new Error(`Expected 0 failures, got ${gen_results.failures.length}`);
	}
	if (!file_generated) {
		throw new Error('Gen function was not called');
	}

	// console.log('✓ All tests passed!');
	process.exit(0);
};

run_test().catch((error) => {
	console.error('Test failed with error:', error); // eslint-disable-line no-console
	process.exit(1);
});
