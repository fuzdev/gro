<script lang="ts">
	import '$routes/fuz.css';
	import '@fuzdev/fuz_code/theme.css';

	import ThemeRoot from '@fuzdev/fuz_ui/ThemeRoot.svelte';
	import type {Snippet} from 'svelte';
	import {Library, library_context} from '@fuzdev/fuz_ui/library.svelte.js';

	import {library_json} from './library.js';

	// TODO add website, rewriting the markdown docs as Svelte

	const {
		children,
	}: {
		children: Snippet;
	} = $props();

	library_context.set(new Library(library_json));
</script>

<svelte:head>
	<title>@fuzdev/gro</title>
</svelte:head>

<ThemeRoot>
	{@render children()}
</ThemeRoot>
