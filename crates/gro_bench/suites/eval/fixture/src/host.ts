/**
 * The host's names, paths, and ports, shared by every phase of the config.
 *
 * @module
 */

/** The service account, its database role, and the SSH login after bootstrap. */
export const DEPLOY_USER = 'harbor';

/** The operators who keep their own logins on the host. */
export const OPERATORS: ReadonlyArray<{ username: string; keys: Array<string> }> = [
	{
		username: 'ana',
		keys: [
			'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBenchFixtureKeyForAnaOnly000000000000000 ana@laptop'
		]
	},
	{
		username: 'tomas',
		keys: [
			'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBenchFixtureKeyForTomasDesk0000000000000 tomas@desk',
			'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBenchFixtureKeyForTomasPhone000000000000 tomas@phone'
		]
	}
];

/** The PostgreSQL database. */
export const DATABASE_NAME = 'harbor';

/** The systemd units, named for the binaries they run. */
export const SERVICE_NAME = 'harbord';
export const WORKER_NAME = 'harbor-worker';

/** The loopback ports nginx proxies to. */
export const PORT = 4870;
export const WORKER_PORT = 4871;

export const HOME_DIR = `/home/${DEPLOY_USER}`;
export const CONFIG_DIR = `${HOME_DIR}/.harbor`;
export const APP_DIR = `${HOME_DIR}/app`;
export const RELEASES_DIR = `${APP_DIR}/releases`;
export const SHARED_DIR = `${APP_DIR}/shared`;
export const CURRENT_LINK = `${APP_DIR}/current`;
export const ENV_FILE = `${SHARED_DIR}/.env.production`;
export const UPLOADS_DIR = `${SHARED_DIR}/uploads`;
export const BACKUPS_DIR = '/var/backups/harbor';
export const TOOLS_DIR = '/opt/harbor-tools';
export const ACME_WEBROOT = '/var/www/acme';

/** The packages every phase leans on, by the name each package manager uses. */
export const BASE_PACKAGES: ReadonlyArray<[id: string, name: string]> = [
	['curl', 'curl'],
	['git', 'git'],
	['unzip', 'unzip'],
	['jq', 'jq'],
	['rsync', 'rsync'],
	['htop', 'htop'],
	['ufw', 'ufw'],
	['fail2ban', 'fail2ban'],
	['nginx', 'nginx'],
	['certbot', 'certbot'],
	['logrotate', 'logrotate'],
	['ca-certificates', 'ca-certificates']
];
