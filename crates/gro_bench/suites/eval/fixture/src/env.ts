/**
 * The service's env file template: the shared fuz_app base, then the
 * service's own lines. Values that differ per instance stay template vars,
 * filled from `zap.env` references where the file is declared.
 *
 * @module
 */

import { fuz_app_env_base, fuz_app_trusted_proxies } from '@fuzdev/zap/recipe_env.ts';

import { CONFIG_DIR, DATABASE_NAME, DEPLOY_USER, PORT, UPLOADS_DIR, WORKER_PORT } from './host.ts';

/** Feature switches, one line each, in the order the service documents them. */
const FEATURES: ReadonlyArray<[name: string, on: boolean, why: string]> = [
	['HARBOR_UPLOADS', true, 'members attach files to their posts'],
	['HARBOR_FEDERATION', false, 'off until the relay is reviewed'],
	['HARBOR_DIGEST_EMAIL', true, 'the weekly digest, sent by the worker'],
	['HARBOR_PUBLIC_SIGNUP', false, 'signup stays invite-only']
];

const feature_lines = (): string =>
	FEATURES.map(([name, on, why]) => `# ${why}\n${name}=${on ? 'true' : 'false'}`).join('\n');

export const ENV_TEMPLATE = `${fuz_app_env_base({
	user: DEPLOY_USER,
	service: DATABASE_NAME,
	config_dir: CONFIG_DIR
})}
${fuz_app_trusted_proxies({ env_prefix: 'HARBOR' })}

# Security - allowed origins for API requests
FUZ_ALLOWED_ORIGINS=https://{{domain}}

# Where the server listens, behind nginx
HARBOR_PORT=${PORT}
HARBOR_WORKER_PORT=${WORKER_PORT}
HARBOR_UPLOADS_DIR=${UPLOADS_DIR}

# Outbound mail, for the digest and invite emails
HARBOR_SMTP_HOST={{smtp_host}}
HARBOR_SMTP_USER={{smtp_user}}
SECRET_HARBOR_SMTP_PASSWORD={{smtp_password}}

${feature_lines()}

# Plain-text logs for journald
NO_COLOR=1
`;
