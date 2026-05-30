<script lang="ts">
	import 'virtual:fuz.css';
	import '@fuzdev/fuz_code/theme.css';

	import ThemeRoot from '@fuzdev/fuz_ui/ThemeRoot.svelte';
	import type {Snippet} from 'svelte';
	import {Library, library_context} from '@fuzdev/fuz_ui/library.svelte.js';
	import {library_json_from_modules} from '@fuzdev/fuz_util/library_json.js';
	import {modules} from 'virtual:svelte-docinfo';

	import package_json from '../../package.json' with {type: 'json'};

	// TODO add website, rewriting the markdown docs as Svelte

	const {
		children,
	}: {
		children: Snippet;
	} = $props();

	const library_json = library_json_from_modules(package_json, modules);

	library_context.set(new Library(library_json));
</script>

<svelte:head>
	<title>@fuzdev/gro</title>
</svelte:head>

<ThemeRoot>
	{@render children()}
</ThemeRoot>
