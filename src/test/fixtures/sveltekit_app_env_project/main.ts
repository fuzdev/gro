// runs in a separate Node process with Gro's loader, cwd set to this fixture project,
// and the env vars passed through the ambient env

/* eslint-disable */

// @ts-ignore
import * as app_env from '$app/env';
// @ts-ignore
import * as env_public from '$app/env/public';
// @ts-ignore
import * as env_private from '$app/env/private';
// a subpath import in the Vite convention, `.js` naming a `.ts` file
import { LIB_PATH } from '#lib/constants.js';

const expected = {
	app_env: { browser: false, building: false, dev: true },
	env_public: { SOME_PUBLIC_VAR: 'public_value' },
	env_private: { SOME_PORT: 3000, SOME_PRIVATE_VAR: 'private_value' } // namespaces sort keys
};

const actual = {
	app_env: { browser: app_env.browser, building: app_env.building, dev: app_env.dev },
	env_public: { ...env_public },
	env_private: { ...env_private }
};

if (LIB_PATH !== 'src/lib') {
	console.error('unexpected LIB_PATH', LIB_PATH);
	process.exit(1);
}

if (JSON.stringify(actual) !== JSON.stringify(expected)) {
	console.error('unexpected env', JSON.stringify(actual));
	process.exit(1);
}
