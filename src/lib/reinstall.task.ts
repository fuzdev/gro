import * as s from '@fuzdev/fuz_schema/schema.ts';

import type { Task } from './task.ts';
import { install_with_cache_healing_or_throw } from './npm_install_helpers.ts';
import { LOCKFILE_FILENAME, NODE_MODULES_DIRNAME } from './constants.ts';
import { fs } from './fs.ts';
import { create_fs_paths } from './fs_paths.ts';

/** @nodocs */
export const Args = s.object({});
export type Args = s.Infer<typeof Args>;

/** @nodocs */
export const task: Task<Args> = {
	summary: `refreshes ${LOCKFILE_FILENAME} with the latest and cleanest deps`,
	Args,
	run: async ({ log, config }): Promise<void> => {
		const fs_paths = create_fs_paths(fs);

		log.info(`running the initial \`${config.pm_cli} install\``);
		await install_with_cache_healing_or_throw(config.pm_cli, { log, context: '(initial)' });

		// Deleting both the lockfile and node_modules upgrades to the latest minor/patch versions.
		await Promise.all([
			fs_paths.remove_path(LOCKFILE_FILENAME),
			fs_paths.remove_path(NODE_MODULES_DIRNAME, { recursive: true })
		]);
		log.info(
			`running \`${config.pm_cli} install\` after deleting ${LOCKFILE_FILENAME} and ${
				NODE_MODULES_DIRNAME
			}, this can take a while...`
		);
		await install_with_cache_healing_or_throw(config.pm_cli, {
			log,
			context: `after deleting ${LOCKFILE_FILENAME} and ${NODE_MODULES_DIRNAME}`
		});

		// TODO @many this relies on npm behavior that changed in v11
		// Deleting the lockfile and reinstalling cleans the lockfile of unnecessary dep noise,
		// like a native tool's many packages for each platform.
		await fs_paths.remove_path(LOCKFILE_FILENAME);
		log.info(`running \`${config.pm_cli} install\` one last time to clean ${LOCKFILE_FILENAME}`);
		await install_with_cache_healing_or_throw(config.pm_cli, { log });
	}
};
