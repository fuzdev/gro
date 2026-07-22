<script lang="ts">
	import DocsContent from '@fuzdev/fuz_ui/DocsContent.svelte';
	import { tomes_context } from '@fuzdev/fuz_ui/tome.ts';
	import { library_context } from '@fuzdev/fuz_ui/library.svelte.ts';

	const get_tomes_by_slug = tomes_context.get();
	const tomes = $derived(Array.from(get_tomes_by_slug().values()));

	const get_library = library_context.get();
	const library = $derived(get_library());
</script>

<DocsContent {tomes} {library}>
	{#snippet repo_name()}
		<h1 class="mb_sm">gro</h1>
	{/snippet}
</DocsContent>
