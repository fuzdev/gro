<script lang="ts">
	import type {Snippet} from 'svelte';
	import Docs from '@fuzdev/fuz_ui/Docs.svelte';
	import {Library, library_context} from '@fuzdev/fuz_ui/library.svelte.js';

	import {tomes} from '$routes/docs/tomes.js';
	import {library_json} from '$routes/library.js';

	const {
		children,
	}: {
		children: Snippet;
	} = $props();

	library_context.set(new Library(library_json));
</script>

<Docs {tomes}>
	{@render children()}
</Docs>
