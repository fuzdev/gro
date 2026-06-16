// Test script to verify run_task passes args and returns output
// This runs in a separate Node.js process with the loader active

/* eslint-disable */

import {Timings} from '@fuzdev/fuz_util/timings.ts';
import {Logger} from '@fuzdev/fuz_util/log.ts';

import {run_task} from '$lib/run_task.ts';
import {load_gro_config} from '$lib/gro_config.ts';
import {Filer} from '$lib/filer.ts';

async function runTest() {
	console.log('Testing run_task passes args and returns output...\n');

	const args = {a: 1, _: []};
	const filer = new Filer();
	const log = new Logger('test');
	const result = await run_task(
		{
			name: 'testTask',
			id: 'foo/testTask',
			mod: {
				task: {
					run: ({args}) => Promise.resolve(args),
				},
			},
		},
		args,
		() => Promise.resolve(),
		await load_gro_config(),
		filer,
		log,
		new Timings(),
	);
	filer.close();

	if (result.ok && result.output === args) {
		console.log('✓ Test passed: run_task returns correct output');
		process.exit(0);
	} else {
		console.error('✗ Test failed: run_task did not return expected output');
		process.exit(1);
	}
}

runTest().catch((error) => {
	console.error('Test failed with error:', error);
	process.exit(1);
});
