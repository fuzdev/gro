import { readdir, rm } from 'node:fs/promises';
import type { RmOptions } from 'node:fs';

import { paths } from './paths.ts';
import {
	NODE_MODULES_DIRNAME,
	GRO_DIST_PREFIX,
	SVELTEKIT_DEV_DIRNAME,
	SVELTEKIT_BUILD_DIRNAME,
	SVELTEKIT_VITE_CACHE_PATH,
	SVELTEKIT_DIST_DIRNAME
} from './constants.ts';

export const clean_fs = async (
	{
		build = false,
		build_dev = false,
		build_dist = false,
		sveltekit = false,
		nodemodules = false
	}: {
		build?: boolean;
		build_dev?: boolean;
		build_dist?: boolean;
		sveltekit?: boolean;
		nodemodules?: boolean;
	},
	rm_options: RmOptions = { force: true, recursive: true }
): Promise<void> => {
	const promises: Array<Promise<void>> = [];

	if (build) {
		promises.push(rm(paths.build, rm_options));
	} else if (build_dev) {
		promises.push(rm(paths.build_dev, rm_options));
	}
	if (build || build_dist) {
		const dir_paths = (await readdir('.')).filter((p) => p.startsWith(GRO_DIST_PREFIX));
		for (const path of dir_paths) {
			promises.push(rm(path, rm_options));
		}
	}
	if (sveltekit) {
		promises.push(rm(SVELTEKIT_DEV_DIRNAME, rm_options));
		promises.push(rm(SVELTEKIT_BUILD_DIRNAME, rm_options));
		promises.push(rm(SVELTEKIT_DIST_DIRNAME, rm_options));
		promises.push(rm(SVELTEKIT_VITE_CACHE_PATH, rm_options));
	}
	if (nodemodules) {
		promises.push(rm(NODE_MODULES_DIRNAME, rm_options));
	}

	await Promise.all(promises);
};
