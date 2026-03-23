<script lang="ts">
	import LibraryDetail from '@fuzdev/fuz_ui/LibraryDetail.svelte';
	import DocsFooter from '@fuzdev/fuz_ui/DocsFooter.svelte';
	import EcosystemLinksPanel from '@fuzdev/fuz_ui/EcosystemLinksPanel.svelte';
	import Breadcrumb from '@fuzdev/fuz_ui/Breadcrumb.svelte';
	import {library_context} from '@fuzdev/fuz_ui/library.svelte.js';

	const library = library_context.get();
</script>

<main class="width_atmost_md pb_xl9">
	<section class="mt_xl3">
		<header class="box">
			<h1>{library.repo_name}</h1>
		</header>
		<Breadcrumb>{library.package_json.glyph}</Breadcrumb>
	</section>
	<EcosystemLinksPanel />
	<section class="box width:100%">
		<div class="panel p_md width_atmost_md">
			<LibraryDetail {library} />
		</div>
	</section>
	<section class="box">
		<nav class="mb_lg">
			<Breadcrumb>{library.package_json.glyph}</Breadcrumb>
		</nav>
		<DocsFooter {library} root_url="https://www.fuz.dev/" />
	</section>
</main>

<style>
	main {
		display: flex;
		flex-direction: column;
		align-items: center;
		margin: 0 auto;
	}
	h1 {
		display: flex;
		align-items: baseline;
		text-align: center;
	}
	section {
		margin-bottom: var(--space_xl5);
	}
</style>
