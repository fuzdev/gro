/**
 * The release push: what `zap deploy` builds locally, ships, and checks.
 *
 * @module
 */

import type { ZapBuilder } from '@fuzdev/zap/zap.ts';

import { APP_DIR, SERVICE_NAME } from './host.ts';

/** The local build, in order: the checks, the server binary, then the site. */
const BUILD_STEPS = [
	'cargo xtask check-release',
	`cargo build --release --bin ${SERVICE_NAME}`,
	'gro build',
	'mkdir -p dist_artifacts',
	`cp target/release/${SERVICE_NAME} dist_artifacts/${SERVICE_NAME}`
];

export const declare_deploy = (zap: ZapBuilder): void => {
	zap.deploy({
		build: BUILD_STEPS.join(' && '),
		artifacts: ['build/', 'dist_artifacts/'],
		path: APP_DIR,
		releases: 5,
		service: SERVICE_NAME,
		shared: ['.env.production', 'uploads'],
		prepare: [`./dist_artifacts/${SERVICE_NAME} migrate`],
		health: { domain: zap.env('DEPLOY_DOMAIN'), path: '/ready', attempts: 20, interval_ms: 500 }
	});
};
