/**
 * Phase 4: nginx in front of the service, with TLS from certbot.
 *
 * The first certificate comes from certbot's standalone mode, which stops
 * nginx briefly on a fresh host; renewal then switches to the webroot, so it
 * runs unattended while nginx serves.
 *
 * @module
 */

import type { ZapBuilder } from '@fuzdev/zap/zap.ts';
import {
	NGINX_VALIDATE,
	RATE_LIMIT_ZONE,
	SSL_PARAMS_PATH,
	nginx_config_verify,
	rate_limit_zone,
	ssl_params
} from '@fuzdev/zap/recipe_nginx.ts';
import {
	debian_nginx_remove_default,
	debian_nginx_spa_site
} from '@fuzdev/zap/recipe_debian_nginx.ts';
import {
	debian_certbot_reload_nginx_hook,
	debian_certbot_stop_nginx
} from '@fuzdev/zap/recipe_debian_certbot.ts';
import { certbot_standalone_issue, certbot_webroot_switch } from '@fuzdev/zap/recipe_certbot.ts';
import { spa_csp } from '@fuzdev/zap/recipe_csp.ts';

import { ACME_WEBROOT, CURRENT_LINK, PORT, WORKER_PORT } from './host.ts';
import { ID } from './ids.ts';

/**
 * The site's CSP: the locked-down baseline, plus the WebSocket and the media
 * host, named by the template's `{{domain}}`, since a template takes no
 * reference.
 */
const CSP = spa_csp({
	websocket: 'wss://{{domain}}',
	img_src_extra: ['https://media.{{domain}}']
});

/** The worker's status page and the upload endpoint, beside the recipe's locations. */
const EXTRA_LOCATIONS = `    # The worker's status, for the operators' dashboard.
    location = /api/worker/status {
        limit_req zone=${RATE_LIMIT_ZONE} burst=5 nodelay;
        proxy_pass http://127.0.0.1:${WORKER_PORT};
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $remote_addr;
    }

    # Uploads carry larger bodies than the API's default.
    location /api/uploads {
        limit_req zone=${RATE_LIMIT_ZONE} burst=10 nodelay;
        client_max_body_size 25m;
        proxy_pass http://127.0.0.1:{{port}};
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $remote_addr;
        proxy_request_buffering off;
    }`;

export const declare_web = (zap: ZapBuilder): void => {
	const domain = zap.env('DEPLOY_DOMAIN');
	const email = zap.env('CERTBOT_EMAIL');
	const mirror = zap.env('MIRROR_DOMAIN');

	zap.directory({
		id: 'acme-webroot',
		name: 'The ACME challenge webroot',
		path: ACME_WEBROOT,
		mode: '755',
		owner: 'root',
		group: 'root'
	});
	zap.script({
		id: ID.certbot_stop_nginx,
		name: 'Stop nginx for the first certificate (fresh host only)',
		...debian_certbot_stop_nginx({ domain }),
		depends_on: ['package-nginx', 'package-certbot']
	});
	zap.script({
		id: ID.certbot_issue,
		name: 'Issue the certificate (standalone, first time only)',
		...certbot_standalone_issue({ domain, extra_domains: [mirror], email }),
		depends_on: [ID.certbot_stop_nginx]
	});
	const reload_hook = debian_certbot_reload_nginx_hook();
	zap.file({
		id: ID.certbot_reload_hook,
		name: 'Reload nginx after a certificate renews',
		...reload_hook,
		depends_on: ['package-certbot']
	});
	zap.script({
		id: ID.certbot_webroot,
		name: 'Renew from the webroot from now on',
		...certbot_webroot_switch({
			domain,
			extra_domains: [mirror],
			email,
			webroot: ACME_WEBROOT,
			deploy_hook: reload_hook
		}),
		depends_on: ['acme-webroot', ID.nginx_service, ID.certbot_reload_hook]
	});

	zap.file({
		id: ID.nginx_ssl_params,
		name: 'TLS parameters',
		path: SSL_PARAMS_PATH,
		content: { type: 'inline', content: ssl_params() },
		mode: '644',
		triggers: [ID.nginx_service],
		validate: NGINX_VALIDATE,
		depends_on: ['package-nginx']
	});
	zap.file({
		id: ID.nginx_rate_limit,
		name: 'The rate-limit zone',
		path: '/etc/nginx/conf.d/rate_limit.conf',
		content: { type: 'inline', content: rate_limit_zone({ rate: '20r/s', size: '20m' }) },
		mode: '644',
		triggers: [ID.nginx_service],
		validate: NGINX_VALIDATE,
		depends_on: ['package-nginx']
	});
	zap.file({
		id: ID.nginx_site,
		name: 'The site',
		path: '/etc/nginx/sites-enabled/harbor.conf',
		content: {
			type: 'template',
			template: debian_nginx_spa_site({
				root: `${CURRENT_LINK}/build`,
				csp: CSP,
				ssl: 'certbot',
				client_max_body_size: '2m',
				api_burst: 40,
				extra_locations: EXTRA_LOCATIONS,
				acme_webroot: ACME_WEBROOT,
				redirect_domains: ['{{mirror}}']
			}),
			vars: { domain, mirror, port: PORT }
		},
		mode: '644',
		triggers: [ID.nginx_service],
		validate: NGINX_VALIDATE,
		depends_on: [ID.certbot_issue, ID.nginx_ssl_params, ID.nginx_rate_limit]
	});
	zap.script({
		id: ID.nginx_remove_default,
		name: "Remove nginx's default site",
		...debian_nginx_remove_default(),
		depends_on: ['package-nginx']
	});
	zap.verify({
		id: ID.nginx_verify,
		name: 'The nginx config is valid',
		...nginx_config_verify(),
		depends_on: [ID.nginx_site, ID.nginx_remove_default]
	});
	zap.service({
		id: ID.nginx_service,
		name: 'nginx running',
		service: 'nginx',
		enabled: true,
		running: true,
		reload_on: [ID.nginx_site, ID.nginx_ssl_params, ID.nginx_rate_limit],
		depends_on: [ID.nginx_verify]
	});
};
