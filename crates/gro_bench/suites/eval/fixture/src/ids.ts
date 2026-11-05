/**
 * Resource ids, one place, so `depends_on` and `triggers` can't drift from
 * the resources they name.
 *
 * @module
 */

export const ID = {
	swap: 'swap-file',
	apt_update: 'apt-update',
	deploy_user: 'user-harbor',
	operator: (username: string): string => `user-${username}`,
	postgres_repo: 'apt-repo-postgres',
	postgres_package: 'package-postgresql',
	database: 'database-harbor',
	bootstrap_token: 'bootstrap-token',
	nginx_site: 'nginx-site',
	nginx_ssl_params: 'nginx-ssl-params',
	nginx_rate_limit: 'nginx-rate-limit',
	nginx_remove_default: 'nginx-remove-default',
	nginx_catch_all: 'nginx-catch-all',
	nginx_verify: 'verify-nginx-config',
	nginx_service: 'service-nginx',
	certbot_issue: 'certbot-issue',
	certbot_webroot: 'certbot-webroot',
	certbot_reload_hook: 'certbot-reload-nginx-hook',
	env_file: 'env-file',
	unit: 'unit-harbord',
	worker_unit: 'unit-harbor-worker',
	service: 'service-harbord',
	worker_service: 'service-harbor-worker',
	firewall: 'firewall',
	fail2ban_jail: 'fail2ban-jail',
	fail2ban_service: 'service-fail2ban',
	verify_firewall: 'verify-post-firewall',
	deno: 'deno-install',
	migrate_tool: 'build-migrate-tool',
	backup_script: 'backup-script',
	backup_cron: 'backup-cron',
	logrotate: 'logrotate-harbor',
	ssh_config: 'ssh-hardening',
	ssh_service: 'service-ssh',
	ssh_verify: 'verify-ssh-hardening',
	verify_pre_hardening: 'verify-pre-hardening'
} as const;
