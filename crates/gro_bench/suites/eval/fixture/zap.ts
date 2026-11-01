/**
 * The bench's eval fixture: a zap config for one host running a web service
 * (harbor, invented for the bench) behind nginx, with PostgreSQL, a
 * background worker, the operators' logins, a firewall, and a release
 * deploy, written against zap's public builder API and its recipes.
 *
 * Instance values are `zap.env` references, so the declaration is the same
 * on every machine. Each phase is its own module, in the order they apply.
 *
 * @module
 */

import type { ZapBuilder } from '@fuzdev/zap/zap.ts';

import { declare_database } from './src/database.ts';
import { declare_deploy } from './src/deploy.ts';
import { declare_firewall } from './src/firewall.ts';
import { declare_foundation } from './src/foundation.ts';
import { declare_hardening } from './src/hardening.ts';
import { DEPLOY_USER } from './src/host.ts';
import { declare_service } from './src/service.ts';
import { declare_tools } from './src/tools.ts';
import { declare_users } from './src/users.ts';
import { declare_web } from './src/web.ts';

export default (zap: ZapBuilder): void => {
	// a fresh host has no service account yet: the first apply connects as
	// `bootstrap_user`, and every later one as the service account
	zap.target({
		host: zap.env('DEPLOY_HOST'),
		user: DEPLOY_USER,
		bootstrap_user: zap.env('BOOTSTRAP_USER'),
		package_manager: 'apt',
		service_manager: 'systemd'
	});
	zap.mask('SMTP_USER');
	zap.unmask('DEPLOY_DOMAIN');

	declare_foundation(zap);
	declare_users(zap);
	declare_database(zap);
	declare_web(zap);
	declare_service(zap);
	declare_firewall(zap);
	declare_tools(zap);
	declare_hardening(zap);
	declare_deploy(zap);
};
