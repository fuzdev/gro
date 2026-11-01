/**
 * Phase 1: the foundation every later phase stands on: swap, the package
 * index, and the base packages.
 *
 * @module
 */

import type { ZapBuilder } from '@fuzdev/zap/zap.ts';

import { BASE_PACKAGES } from './host.ts';
import { ID } from './ids.ts';

export const declare_foundation = (zap: ZapBuilder): void => {
	// a small host can run out of memory during an install or a build
	zap.swap({ id: ID.swap, name: '2GB swap file', size: '2G' });

	zap.script({
		id: ID.apt_update,
		name: 'Refresh the package index once a day',
		check: 'test -n "$(find /var/lib/apt/lists -maxdepth 0 -mmin -1440)"',
		run: 'sudo apt-get update -q'
	});

	for (const [id, name] of BASE_PACKAGES) {
		zap.pkg({
			id: `package-${id}`,
			name: `Install ${name}`,
			from: { apt: name, apk: name },
			depends_on: [ID.apt_update]
		});
	}
};
