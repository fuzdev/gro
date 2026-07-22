<script lang="ts">
	import type { Snippet } from 'svelte';
	import Docs from '@fuzdev/fuz_ui/Docs.svelte';
	import { Library, library_context } from '@fuzdev/fuz_ui/library.svelte.ts';

	import { tomes } from '$routes/docs/tomes.ts';
	import { library_json } from '$routes/library.ts';

	const {
		children
	}: {
		children: Snippet;
	} = $props();

	const library = new Library(library_json);
	library_context.set(() => library);
</script>

<Docs {tomes}>
	{@render children()}
</Docs>
