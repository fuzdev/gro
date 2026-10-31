// Test script to verify run_task handles failing tasks
// This runs in a separate Node.js process with the loader active

/* eslint-disable */

import { Timings } from '@fuzdev/fuz_util/timings.ts';
import { Logger } from '@fuzdev/fuz_util/log.ts';

import { run_task } from '#lib/run_task.ts';
import { load_gro_config } from '#lib/gro_config.ts';
import { create_node_module_graph, create_node_watcher } from '#lib/module_graph_node.ts';

async function runTest() {
	console.log('Testing run_task handles failing tasks...\n');

	let err;
	const config = await load_gro_config();
	const graph = create_node_module_graph({ watch: config.watch });
	const watcher = create_node_watcher({ watch: config.watch });
	const log = new Logger('test');
	const result = await run_task(
		{
			name: 'testTask',
			id: 'foo/testTask',
			mod: {
				task: {
					run: () => {
						err = Error('Test error');
						throw err;
					}
				}
			}
		},
		{ _: [] },
		{
			invoke_task: async () => {},
			config,
			graph,
			watcher,
			log,
			timings: new Timings()
		}
	);
	await graph.close();
	await watcher.close();

	const success = !result.ok && result.reason && result.error === err;

	if (success) {
		console.log('✓ Test passed: run_task correctly handles failing tasks');
		process.exit(0);
	} else {
		console.error('✗ Test failed: run_task did not correctly handle failing task');
		console.error('Result:', result);
		console.error('Expected error:', err);
		process.exit(1);
	}
}

runTest().catch((error) => {
	console.error('Test failed with error:', error);
	process.exit(1);
});
