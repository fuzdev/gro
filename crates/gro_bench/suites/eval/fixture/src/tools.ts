/**
 * Phase 7: the operators' tools: Deno for the maintenance scripts, the
 * migration tool built from a pinned release, the nightly backup, and log
 * rotation.
 *
 * @module
 */

import type { ZapBuilder } from '@fuzdev/zap/zap.ts';
import { deno_install } from '@fuzdev/zap/recipe_deno.ts';

import { BACKUPS_DIR, DATABASE_NAME, DEPLOY_USER, TOOLS_DIR, UPLOADS_DIR } from './host.ts';
import { ID } from './ids.ts';

const DENO_VERSION = '2.8.3';
const MIGRATE_VERSION = '0.14.2';
/** The release archive's BLAKE3, pinned with its version. */
const MIGRATE_BLAKE3 = 'a3f1c2e4b5d6978810213243546576879a0b1c2d3e4f5061728394a5b6c7d8e9';

/** How the maintenance steps reach the database. */
const PG_ARGS = `-h 127.0.0.1 -U ${DEPLOY_USER} -d ${DATABASE_NAME}`;

/** How many nightly dumps to keep. */
const BACKUPS_KEPT = 14;

const BACKUP_SCRIPT = `#!/bin/sh
# Nightly dump of the ${DATABASE_NAME} database and the uploads, keeping ${BACKUPS_KEPT}.
set -eu
stamp="$(date -u +%Y%m%dT%H%M%SZ)"
pg_dump -h 127.0.0.1 -U ${DEPLOY_USER} -Fc ${DATABASE_NAME} > "${BACKUPS_DIR}/db-$stamp.dump"
tar -C "${UPLOADS_DIR}" -czf "${BACKUPS_DIR}/uploads-$stamp.tar.gz" .
ls -1t "${BACKUPS_DIR}"/db-*.dump | tail -n +${BACKUPS_KEPT + 1} | xargs -r rm --
ls -1t "${BACKUPS_DIR}"/uploads-*.tar.gz | tail -n +${BACKUPS_KEPT + 1} | xargs -r rm --
`;

const LOGROTATE = `${BACKUPS_DIR}/*.log {
    weekly
    rotate 8
    compress
    missingok
    notifempty
}
`;

export const declare_tools = (zap: ZapBuilder): void => {
	zap.script({
		id: ID.deno,
		name: `Deno ${DENO_VERSION} for the maintenance scripts`,
		...deno_install({ version: DENO_VERSION, bin_path: `${TOOLS_DIR}/bin`, privileged: true }),
		depends_on: ['package-curl', 'package-unzip']
	});

	zap.build({
		id: ID.migrate_tool,
		name: `The migration tool ${MIGRATE_VERSION}`,
		source: {
			type: 'remote',
			url: `https://releases.example.org/harbor-migrate/v${MIGRATE_VERSION}/harbor-migrate-${MIGRATE_VERSION}.tar.gz`,
			extract: { type: 'tar.gz', strip_components: 1 },
			blake3: MIGRATE_BLAKE3
		},
		run: 'make release',
		output: `${TOOLS_DIR}/bin/harbor-migrate`,
		env: { HARBOR_MIGRATE_FEATURES: 'postgres' }
	});

	zap.directory({
		id: 'dir-backups',
		name: 'The backups directory',
		path: BACKUPS_DIR,
		mode: '750',
		owner: DEPLOY_USER,
		group: DEPLOY_USER,
		depends_on: [ID.deploy_user]
	});
	zap.file({
		id: ID.backup_script,
		name: 'The nightly backup script',
		path: `${TOOLS_DIR}/bin/harbor-backup`,
		content: { type: 'inline', content: BACKUP_SCRIPT },
		mode: '755',
		depends_on: ['dir-backups']
	});
	zap.file({
		id: ID.backup_cron,
		name: 'Run the backup nightly',
		path: '/etc/cron.d/harbor-backup',
		content: {
			type: 'inline',
			content: `17 3 * * * ${DEPLOY_USER} ${TOOLS_DIR}/bin/harbor-backup >> ${BACKUPS_DIR}/backup.log 2>&1\n`
		},
		mode: '644',
		depends_on: [ID.backup_script]
	});
	zap.file({
		id: ID.logrotate,
		name: 'Rotate the backup log',
		path: '/etc/logrotate.d/harbor',
		content: { type: 'inline', content: LOGROTATE },
		mode: '644',
		depends_on: ['package-logrotate']
	});
	zap.link({
		id: 'link-backup-tool',
		name: 'The backup script on PATH',
		path: '/usr/local/bin/harbor-backup',
		target: `${TOOLS_DIR}/bin/harbor-backup`,
		depends_on: [ID.backup_script]
	});

	zap.pipeline({
		id: 'pipeline-maintenance',
		name: 'Vacuum and check the database after an apply',
		steps: [
			{ id: 'vacuum', run: `psql ${PG_ARGS} -c 'VACUUM (ANALYZE)'` },
			{
				id: 'migrations',
				run: `${TOOLS_DIR}/bin/harbor-migrate status --database ${DATABASE_NAME}`
			},
			{ id: 'report', run: `${TOOLS_DIR}/bin/deno run --allow-read ${TOOLS_DIR}/report.ts` }
		],
		depends_on: [ID.database, ID.migrate_tool, ID.deno]
	});
};
