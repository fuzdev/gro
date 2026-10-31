// Test script to verify run_task passes args and returns output
// This runs in a separate Node.js process with the loader active

/* eslint-disable */

import { Timings } from '@fuzdev/fuz_util/timings.ts';
import { Logger } from '@fuzdev/fuz_util/log.ts';

import { run_task } from '#lib/run_task.ts';
import { load_gro_config } from '#lib/gro_config.ts';
import { create_node_module_graph, create_node_watcher } from '#lib/module_graph_node.ts';

async function runTest() {
	console.log('Testing run_task passes args and returns output...\n');

	const args = { a: 1, _: [] };
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
					run: ({ args }) => Promise.resolve(args)
				}
			}
		},
		args,
		() => Promise.resolve(),
		config,
		graph,
		watcher,
		log,
		new Timings()
	);
	await graph.close();
	await watcher.close();

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
