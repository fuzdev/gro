import {defineConfig} from 'vite';
import {sveltekit} from '@sveltejs/kit/vite';
import {vite_plugin_library_well_known} from '@fuzdev/fuz_ui/vite_plugin_library_well_known.js';

export default defineConfig({
	plugins: [sveltekit(), vite_plugin_library_well_known()],
	optimizeDeps: {exclude: ['@fuzdev/blake3_wasm']},
});
