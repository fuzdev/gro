import {z} from 'zod';
import {rm} from 'node:fs/promises';

import type {Task} from './task.ts';
import {install_with_cache_healing_or_throw} from './npm_install_helpers.ts';
import {LOCKFILE_FILENAME, NODE_MODULES_DIRNAME} from './constants.ts';

/** @nodocs */
export const Args = z.strictObject({});
export type Args = z.infer<typeof Args>;

/** @nodocs */
export const task: Task<Args> = {
	summary: `refreshes ${LOCKFILE_FILENAME} with the latest and cleanest deps`,
	Args,
	run: async ({log, config}): Promise<void> => {
		log.info(`running the initial \`${config.pm_cli} install\``);
		await install_with_cache_healing_or_throw(config.pm_cli, {log, context: '(initial)'});

		// Deleting both the lockfile and node_modules upgrades to the latest minor/patch versions.
		await Promise.all([rm(LOCKFILE_FILENAME), rm(NODE_MODULES_DIRNAME, {recursive: true})]);
		log.info(
			`running \`${config.pm_cli} install\` after deleting ${LOCKFILE_FILENAME} and ${
				NODE_MODULES_DIRNAME
			}, this can take a while...`,
		);
		await install_with_cache_healing_or_throw(config.pm_cli, {
			log,
			context: `after deleting ${LOCKFILE_FILENAME} and ${NODE_MODULES_DIRNAME}`,
		});

		// TODO @many this relies on npm behavior that changed in v11
		// Deleting the lockfile and reinstalling cleans the lockfile of unnecessary dep noise,
		// like esbuild's many packages for each platform.
		await rm(LOCKFILE_FILENAME);
		log.info(`running \`${config.pm_cli} install\` one last time to clean ${LOCKFILE_FILENAME}`);
		await install_with_cache_healing_or_throw(config.pm_cli, {log});
	},
};
