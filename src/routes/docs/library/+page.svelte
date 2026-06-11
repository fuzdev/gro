<script lang="ts">
	import LibraryDetail from '@fuzdev/fuz_ui/LibraryDetail.svelte';
	import TomeContent from '@fuzdev/fuz_ui/TomeContent.svelte';
	import {tome_get_by_slug} from '@fuzdev/fuz_ui/tome.js';
	import {library_context} from '@fuzdev/fuz_ui/library.svelte.js';

	const tome = tome_get_by_slug('library');

	const get_library = library_context.get();
	const library = $derived(get_library());
</script>

<TomeContent {tome}>
	<section>
		<LibraryDetail {library} />
	</section>
</TomeContent>
