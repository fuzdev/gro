/**
 * Phase 2: the service account and the operators' own logins.
 *
 * The service account gets root's keys at bootstrap, so a fresh host has a
 * second way in before root login closes; each operator gets their own keys
 * and sudo for the service's units only.
 *
 * @module
 */

import type { ZapBuilder } from '@fuzdev/zap/zap.ts';

import { DEPLOY_USER, OPERATORS, SERVICE_NAME, WORKER_NAME } from './host.ts';
import { ID } from './ids.ts';

/** The commands an operator may run with sudo: the service's units, and logs. */
const operator_commands = (): Array<string> =>
	[SERVICE_NAME, WORKER_NAME].flatMap((unit) => [
		`/usr/bin/systemctl restart ${unit}`,
		`/usr/bin/systemctl status ${unit}`,
		`/usr/bin/journalctl -u ${unit}`
	]);

export const declare_users = (zap: ZapBuilder): void => {
	zap.user({
		id: ID.deploy_user,
		name: `Create the ${DEPLOY_USER} user with root's SSH keys and sudo`,
		username: DEPLOY_USER,
		shell: '/bin/bash',
		// lets nginx traverse the home directory to the release's static files
		home_mode: '711',
		ssh_keys: { copy_from: 'root' },
		sudo: 'nopasswd'
	});

	for (const { username, keys } of OPERATORS) {
		zap.user({
			id: ID.operator(username),
			name: `Create the operator ${username}`,
			username,
			shell: '/bin/bash',
			ssh_keys: { keys },
			sudo: { mode: 'nopasswd', commands: operator_commands() },
			groups: ['adm', 'systemd-journal']
		});
	}
};
