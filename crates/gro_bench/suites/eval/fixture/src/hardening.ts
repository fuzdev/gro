/**
 * Phase 8, last: close root login and password auth, once a check confirms
 * the service account and the operators can still get in.
 *
 * @module
 */

import type { ZapBuilder } from '@fuzdev/zap/zap.ts';
import { SSHD_VALIDATE, ssh_hardening, ssh_hardening_verify } from '@fuzdev/zap/recipe_ssh.ts';
import { debian_ssh_service } from '@fuzdev/zap/recipe_debian_ssh.ts';

import { DEPLOY_USER, OPERATORS } from './host.ts';
import { ID } from './ids.ts';

export const declare_hardening = (zap: ZapBuilder): void => {
	const logins = [DEPLOY_USER, ...OPERATORS.map((operator) => operator.username)];
	zap.verify({
		id: ID.verify_pre_hardening,
		name: 'Every login keeps its keys before root login closes',
		assertions: logins.flatMap((username) => [
			{
				type: 'path_exists' as const,
				path: `/home/${username}/.ssh/authorized_keys`,
				file_type: 'file' as const,
				owner: username,
				not_empty: true
			},
			{ type: 'command' as const, run: `id -u ${username}` }
		]),
		depends_on: [ID.verify_firewall, ...OPERATORS.map((operator) => ID.operator(operator.username))]
	});

	// One options object for the drop-in and its verify, so the two can't drift
	const hardening = { user: DEPLOY_USER, max_auth_tries: 3, login_grace_time: 30 };
	zap.file({
		id: ID.ssh_config,
		name: 'SSH hardening',
		path: '/etc/ssh/sshd_config.d/50-harbor.conf',
		content: {
			type: 'inline',
			content: ssh_hardening(hardening)
		},
		mode: '644',
		validate: SSHD_VALIDATE,
		depends_on: [ID.verify_pre_hardening]
	});
	zap.service({
		id: ID.ssh_service,
		name: 'sshd',
		...debian_ssh_service({ reload_on: [ID.ssh_config] }),
		depends_on: [ID.ssh_config]
	});
	zap.verify({
		id: ID.ssh_verify,
		name: 'Effective sshd config is hardened',
		...ssh_hardening_verify(hardening),
		depends_on: [ID.ssh_service]
	});
};
