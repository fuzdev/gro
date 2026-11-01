/**
 * Phase 6: the firewall and fail2ban, then a check that SSH still answers.
 *
 * @module
 */

import type { ZapBuilder } from '@fuzdev/zap/zap.ts';
import { debian_web_firewall } from '@fuzdev/zap/recipe_debian_firewall.ts';
import { fail2ban_jail } from '@fuzdev/zap/recipe_fail2ban.ts';

import { DEPLOY_USER } from './host.ts';
import { ID } from './ids.ts';

/** The TURN relay's ports, beside the web ports the recipe opens. */
const TURN_PORTS = [
	{ port: 3478, proto: 'udp' },
	{ port: 3478, proto: 'tcp' },
	{ port: 5349, proto: 'tcp' }
] as const;

export const declare_firewall = (zap: ZapBuilder): void => {
	zap.firewall({
		id: ID.firewall,
		name: 'ufw: SSH, the web, and TURN',
		...debian_web_firewall({ allow_extra: TURN_PORTS }),
		depends_on: ['package-ufw', ID.deploy_user]
	});

	zap.verify({
		id: ID.verify_firewall,
		name: 'SSH still answers after the firewall',
		assertions: [
			{ type: 'command', run: 'sudo ufw status | head -n 1', expect: 'Status: active' },
			{ type: 'command', run: 'sudo ss -tlnp | grep -c sshd', expect: '1' },
			{ type: 'path_exists', path: `/home/${DEPLOY_USER}/.ssh/authorized_keys`, not_empty: true }
		],
		depends_on: [ID.firewall]
	});

	zap.file({
		id: ID.fail2ban_jail,
		name: 'fail2ban jail',
		path: '/etc/fail2ban/jail.local',
		content: {
			type: 'inline',
			content: fail2ban_jail({ ban_time: '2h', max_retry: 4, sshd_max_retry: 3 })
		},
		mode: '644',
		triggers: [ID.fail2ban_service],
		depends_on: ['package-fail2ban']
	});
	zap.service({
		id: ID.fail2ban_service,
		name: 'fail2ban running',
		service: 'fail2ban',
		enabled: true,
		running: true,
		restart_on: [ID.fail2ban_jail],
		depends_on: [ID.fail2ban_jail]
	});
};
