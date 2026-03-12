<script lang="ts">
	import DocsContent from '@fuzdev/fuz_ui/DocsContent.svelte';
	import {tomes_context} from '@fuzdev/fuz_ui/tome.js';
	import {library_context} from '@fuzdev/fuz_ui/library.svelte.js';

	const tomes_by_name = tomes_context.get();

	const tomes = $derived(Array.from(tomes_by_name().values()));

	const library = library_context.get();
</script>

<DocsContent {tomes} {library}>
	{#snippet repo_name()}
		<h1 class="mb_sm">gro</h1>
	{/snippet}
</DocsContent>
