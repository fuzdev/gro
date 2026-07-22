import { defineConfig } from 'vite';
import { sveltekit } from '@sveltejs/kit/vite';
import { vite_plugin_fuz_css } from '@fuzdev/fuz_css/vite_plugin_fuz_css.ts';
import svelte_docinfo from 'svelte-docinfo/vite.js';
import { vite_plugin_pkg_json } from '@fuzdev/fuz_ui/vite_plugin_pkg_json.ts';

export default defineConfig({
	plugins: [sveltekit(), svelte_docinfo(), vite_plugin_fuz_css(), vite_plugin_pkg_json()],
	optimizeDeps: { exclude: ['@fuzdev/blake3_wasm'] }
});
