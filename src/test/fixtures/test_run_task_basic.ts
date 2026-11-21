// Test script to verify run_task passes args and returns output
// This runs in a separate Node.js process with the loader active

/* eslint-disable */

import { Timings } from '@fuzdev/fuz_util/timings.ts';
import { Logger } from '@fuzdev/fuz_util/log.ts';

import { run_task } from '#lib/run_task.ts';
import { load_gro_config } from '#lib/gro_config.ts';
import { fs } from '#lib/fs.ts';
import { create_fs_paths } from '#lib/fs_paths.ts';
import { cwd } from '#lib/cwd.ts';
import { env } from '#lib/env.ts';
import { create_node_module_graph } from '#lib/module_graph_node.ts';
import { spawn } from '#lib/spawn.ts';
import { stdout } from '#lib/stdout.ts';
import { stderr } from '#lib/stderr.ts';
import { import_module_node } from '#lib/internal/node_import_module.ts';
import { create_node_platform } from '#lib/internal/node_platform.ts';

async function runTest() {
	console.log('Testing run_task passes args and returns output...\n');

	const args = { a: 1, _: [] };
	const fs_paths = create_fs_paths(fs, { cwd: cwd(), home: env.get('HOME') });
	const config = await load_gro_config(fs_paths, import_module_node);
	const graph = create_node_module_graph({ watch: config.watch });
	const log = new Logger('test');
	const result = await run_task(
		{
			name: 'testTask',
			id: 'foo/testTask',
			mod: {
				task: {
					run: ({ task_args }) => Promise.resolve(task_args)
				}
			}
		},
		args,
		{
			invoke_task: () => Promise.resolve(),
			config,
			fs_paths,
			raw_rest_args: [],
			import_module: import_module_node,
			host_root: { env, spawn, stdout, stderr },
			platform: await create_node_platform(fs_paths),
			implicit_rest_args: [],
			grants: null,
			graph,
			log,
			timings: new Timings()
		}
	);
	await graph.close();

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
