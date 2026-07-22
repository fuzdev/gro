import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';
import adapter from '@sveltejs/adapter-static';
import { svelte_preprocess_mdz } from '@fuzdev/mdz/svelte_preprocess_mdz.js';
import { svelte_preprocess_fuz_code } from '@fuzdev/fuz_code/svelte_preprocess_fuz_code.js';
import { create_csp_directives } from '@fuzdev/fuz_ui/csp.js';
import { csp_directives_of_fuzdev } from '@fuzdev/fuz_ui/csp_of_fuzdev.js';
import { execSync } from 'node:child_process';

/** @type {import('@sveltejs/kit').Config} */
export default {
	preprocess: [svelte_preprocess_mdz(), svelte_preprocess_fuz_code(), vitePreprocess()],
	compilerOptions: { runes: true },
	vitePlugin: { inspector: true },
	kit: {
		adapter: adapter(),
		paths: { relative: false }, // use root-absolute paths for SSR path comparison: https://svelte.dev/docs/kit/configuration#paths
		alias: { $routes: 'src/routes', '@fuzdev/gro': 'src/lib' },
		csp: {
			directives: create_csp_directives({
				extend: [csp_directives_of_fuzdev]
			})
		},
		version: {
			name: execSync('git rev-parse HEAD').toString().trim()
		}
	}
};
