import {z} from 'zod';
import {spawn} from '@fuzdev/fuz_util/process.js';

import {TaskError, type Task} from './task.ts';
import {package_json_sync} from './package_json.ts';
import {sveltekit_sync} from './sveltekit_helpers.ts';

/**
 * Env vars that tell npm/yarn/pnpm to skip devDependencies. We strip these
 * from the install subprocess env because gro/sync's install step exists to
 * make the project buildable — and the build needs devDeps (TypeScript,
 * Vite, svelte-kit's adapter packages, etc.). Inheriting `NODE_ENV=production`
 * from the shell prunes those packages and then `gro build` fails on the
 * very next step with an opaque "Cannot find package" error.
 *
 * Users who genuinely want a production-pruned install run it themselves
 * (e.g. `npm ci --omit=dev`) outside the gro pipeline.
 */
const PRUNE_TRIGGERING_ENV_VARS: ReadonlySet<string> = new Set([
	'NODE_ENV',
	'npm_config_production',
	'npm_config_omit',
]);

export const sanitize_install_env = (env: NodeJS.ProcessEnv): NodeJS.ProcessEnv => {
	const result: NodeJS.ProcessEnv = {};
	for (const [key, value] of Object.entries(env)) {
		if (PRUNE_TRIGGERING_ENV_VARS.has(key)) continue;
		result[key] = value;
	}
	return result;
};

/** @nodocs */
export const Args = z.strictObject({
	sveltekit: z.boolean().meta({description: 'dual of no-sveltekit'}).default(true),
	'no-sveltekit': z.boolean().meta({description: 'opt out of svelte-kit sync'}).default(false),
	package_json: z.boolean().meta({description: 'dual of no-package_json'}).default(true),
	'no-package_json': z.boolean().meta({description: 'opt out of package.json sync'}).default(false),
	gen: z.boolean().meta({description: 'dual of no-gen'}).default(true),
	'no-gen': z.boolean().meta({description: 'opt out of running gen'}).default(false),
	install: z.boolean().meta({description: 'opt into installing packages'}).default(false),
});
export type Args = z.infer<typeof Args>;

/** @nodocs */
export const task: Task<Args> = {
	summary: 'run `gro gen`, update `package.json`, and optionally install packages to sync up',
	Args,
	run: async ({args, invoke_task, config, log}): Promise<void> => {
		const {sveltekit, package_json, gen, install} = args;

		if (install) {
			const result = await spawn(config.pm_cli, ['install'], {
				env: sanitize_install_env(process.env),
			});
			if (!result.ok) {
				throw new TaskError(`Failed \`${config.pm_cli} install\``);
			}
		}

		if (sveltekit) {
			await sveltekit_sync(undefined, config.pm_cli);
			log.info('synced SvelteKit');
		}

		if (package_json && config.map_package_json) {
			await package_json_sync(config.map_package_json, log);
		}

		if (gen) {
			await invoke_task('gen');
		}
	},
};
