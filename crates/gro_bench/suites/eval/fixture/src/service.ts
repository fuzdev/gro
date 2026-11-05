/**
 * Phase 5: the service and its worker, each a hardened systemd unit reading
 * the shared env file, and the directories the releases live in.
 *
 * @module
 */

import type { ZapBuilder } from '@fuzdev/zap/zap.ts';
import { hardened_systemd_unit } from '@fuzdev/zap/recipe_debian_systemd.ts';

import { env_content } from './env.ts';
import {
	APP_DIR,
	CONFIG_DIR,
	CURRENT_LINK,
	DEPLOY_USER,
	ENV_FILE,
	PORT,
	RELEASES_DIR,
	SERVICE_NAME,
	SHARED_DIR,
	UPLOADS_DIR,
	WORKER_NAME
} from './host.ts';
import { ID } from './ids.ts';

/** The directories the service owns, each with its mode. */
const DIRECTORIES: ReadonlyArray<[id: string, path: string, mode: string]> = [
	['dir-config', CONFIG_DIR, '700'],
	['dir-app', APP_DIR, '755'],
	['dir-releases', RELEASES_DIR, '755'],
	['dir-shared', SHARED_DIR, '750'],
	['dir-uploads', UPLOADS_DIR, '750']
];

/** One unit of the two: the server, or the worker beside it. */
interface Unit {
	id: string;
	service_id: string;
	name: string;
	description: string;
	exec_start: string;
	read_write_paths: Array<string>;
}

const UNITS: ReadonlyArray<Unit> = [
	{
		id: ID.unit,
		service_id: ID.service,
		name: SERVICE_NAME,
		description: 'harbor server',
		exec_start: `${CURRENT_LINK}/dist_artifacts/${SERVICE_NAME}`,
		// the server deletes the bootstrap token once it's claimed, and writes uploads
		read_write_paths: [CONFIG_DIR, UPLOADS_DIR]
	},
	{
		id: ID.worker_unit,
		service_id: ID.worker_service,
		name: WORKER_NAME,
		description: 'harbor background worker',
		exec_start: `${CURRENT_LINK}/dist_artifacts/${SERVICE_NAME} worker`,
		read_write_paths: [UPLOADS_DIR]
	}
];

export const declare_service = (zap: ZapBuilder): void => {
	for (const [id, path, mode] of DIRECTORIES) {
		zap.directory({
			id,
			name: `The directory ${path}`,
			path,
			mode,
			owner: DEPLOY_USER,
			group: DEPLOY_USER,
			depends_on: [ID.deploy_user]
		});
	}

	zap.file({
		id: ID.env_file,
		name: 'The service env file',
		path: ENV_FILE,
		content: env_content({
			domain: zap.env('DEPLOY_DOMAIN'),
			cookie_keys: zap.env('SECRET_FUZ_COOKIE_KEYS'),
			smtp_host: zap.env('SMTP_HOST'),
			smtp_user: zap.env('SMTP_USER'),
			smtp_password: zap.env('SECRET_SMTP_PASSWORD')
		}),
		mode: '600',
		owner: DEPLOY_USER,
		group: DEPLOY_USER,
		triggers: UNITS.map((unit) => unit.service_id),
		depends_on: ['dir-shared']
	});

	for (const unit of UNITS) {
		zap.file({
			id: unit.id,
			name: `The ${unit.name} unit`,
			path: `/etc/systemd/system/${unit.name}.service`,
			content: {
				type: 'inline',
				content: hardened_systemd_unit({
					description: unit.description,
					exec_start: unit.exec_start,
					user: DEPLOY_USER,
					working_directory: CURRENT_LINK,
					read_write_paths: unit.read_write_paths,
					env_file: ENV_FILE,
					after: ['network-online.target', 'postgresql.service'],
					wants: ['network-online.target'],
					restart: 'on-failure',
					restart_sec: 5
				})
			},
			mode: '644',
			triggers: [unit.service_id],
			depends_on: [ID.env_file]
		});
		zap.service({
			id: unit.service_id,
			name: `${unit.name} running`,
			service: unit.name,
			enabled: true,
			running: true,
			restart_on: [unit.id, ID.env_file],
			depends_on: [unit.id, ID.database, ID.bootstrap_token]
		});
	}

	zap.verify({
		id: 'verify-service-listening',
		name: `${SERVICE_NAME} answers on loopback`,
		assertions: [
			{ type: 'path_exists', path: CURRENT_LINK, owner: DEPLOY_USER },
			{ type: 'path_exists', path: ENV_FILE, file_type: 'file', mode: '600', not_empty: true },
			{
				type: 'command',
				run: `curl -fsS http://127.0.0.1:${PORT}/health`,
				expect: 'ok'
			}
		],
		depends_on: [ID.service]
	});
};
