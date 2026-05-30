<script lang="ts">
	import 'virtual:fuz.css';
	import '@fuzdev/fuz_code/theme.css';

	import ThemeRoot from '@fuzdev/fuz_ui/ThemeRoot.svelte';
	import type {Snippet} from 'svelte';
	import {SiteState, site_context} from '@fuzdev/fuz_ui/site.svelte.js';
	import {logo_gro} from '@fuzdev/fuz_ui/logos.js';

	// TODO add website, rewriting the markdown docs as Svelte

	const {
		children,
	}: {
		children: Snippet;
	} = $props();

	site_context.set(
		new SiteState({
			icon: logo_gro,
			glyph: '🌰',
			repo_url: 'https://github.com/fuzdev/gro',
		}),
	);
</script>

<svelte:head>
	<title>@fuzdev/gro</title>
</svelte:head>

<ThemeRoot>
	{@render children()}
</ThemeRoot>
