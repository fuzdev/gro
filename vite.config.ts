import {defineConfig} from 'vite';
import {sveltekit} from '@sveltejs/kit/vite';
import {vite_plugin_fuz_css} from '@fuzdev/fuz_css/vite_plugin_fuz_css.js';
import svelte_docinfo from 'svelte-docinfo/vite.js';

export default defineConfig({
	plugins: [vite_plugin_fuz_css(), sveltekit(), svelte_docinfo()],
	optimizeDeps: {exclude: ['@fuzdev/blake3_wasm']},
});
