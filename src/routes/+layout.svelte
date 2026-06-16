<script lang="ts">
	import 'virtual:fuz.css';
	import '@fuzdev/fuz_code/theme.css';

	import ThemeRoot from '@fuzdev/fuz_ui/ThemeRoot.svelte';
	import type {Snippet} from 'svelte';
	import {SiteState, site_context} from '@fuzdev/fuz_ui/site.svelte.ts';
	import {logo_gro} from '@fuzdev/fuz_ui/logos.ts';
	import pkg_json from 'virtual:pkg.json';

	// TODO add website, rewriting the markdown docs as Svelte

	const {
		children,
	}: {
		children: Snippet;
	} = $props();

	site_context.set(new SiteState({icon: logo_gro, pkg_json}));
</script>

<svelte:head>
	<title>@fuzdev/gro</title>
</svelte:head>

<ThemeRoot>
	{@render children()}
</ThemeRoot>
