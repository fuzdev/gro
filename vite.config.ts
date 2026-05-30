import {defineConfig} from 'vite';
import {sveltekit} from '@sveltejs/kit/vite';
import svelte_docinfo from 'svelte-docinfo/vite.js';

export default defineConfig({
	plugins: [sveltekit(), svelte_docinfo()],
	optimizeDeps: {exclude: ['@fuzdev/blake3_wasm']},
});
