/**
 * Phase 3: PostgreSQL from the upstream repository, the service's database
 * with its loopback TCP auth rule, and the one-shot bootstrap token.
 *
 * @module
 */

import type { ZapBuilder } from '@fuzdev/zap/zap.ts';
import { postgres_apt_repo } from '@fuzdev/zap/recipe_debian_postgres.ts';
import { fuz_app_bootstrap_token } from '@fuzdev/zap/recipe_fuz_app_bootstrap.ts';

import { CONFIG_DIR, DATABASE_NAME, DEPLOY_USER } from './host.ts';
import { ID } from './ids.ts';

/** The major version the service is tested against. */
const POSTGRES_MAJOR = 17;

export const declare_database = (zap: ZapBuilder): void => {
	zap.apt_repo({
		id: ID.postgres_repo,
		name: 'PostgreSQL apt repository',
		...postgres_apt_repo(),
		depends_on: [ID.apt_update]
	});

	zap.pkg({
		id: ID.postgres_package,
		name: `Install PostgreSQL ${POSTGRES_MAJOR}`,
		from: { apt: `postgresql-${POSTGRES_MAJOR}`, apk: `postgresql${POSTGRES_MAJOR}` },
		check: `psql --version | grep -q ' ${POSTGRES_MAJOR}\\.'`,
		depends_on: [ID.postgres_repo]
	});

	zap.database({
		id: ID.database,
		name: `Create the ${DATABASE_NAME} database`,
		engine: 'postgresql',
		database_name: DATABASE_NAME,
		username: DEPLOY_USER,
		auth: { method: 'scram-sha-256', hosts: ['127.0.0.1/32', '::1/128'] },
		depends_on: [ID.postgres_package, ID.deploy_user]
	});

	zap.script({
		id: ID.bootstrap_token,
		name: 'Write the one-shot bootstrap token',
		...fuz_app_bootstrap_token({
			user: DEPLOY_USER,
			service: DATABASE_NAME,
			config_dir: CONFIG_DIR,
			psql_args: '-h 127.0.0.1'
		}),
		depends_on: [ID.database]
	});
};
