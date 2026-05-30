<script lang="ts">
	import {resolve} from '$app/paths';
	import DocsFooter from '@fuzdev/fuz_ui/DocsFooter.svelte';
	import {site_context} from '@fuzdev/fuz_ui/site.svelte.js';
	import {FUZ_DEV_URL} from '@fuzdev/fuz_ui/constants.js';
	import Svg from '@fuzdev/fuz_ui/Svg.svelte';
	import Card from '@fuzdev/fuz_ui/Card.svelte';
	import {logo_gro} from '@fuzdev/fuz_ui/logos.js';

	const site = site_context.get();
</script>

<main class="box width:100%">
	<div class="box width_atmost_md">
		<section class="box">
			<h1>gro</h1>
			<a class="panel p_md box mb_xl3" title="source repo" href="https://github.com/fuzdev/gro">
				<Svg data={logo_gro} size="var(--icon_size_lg)" />
			</a>
		</section>
		<section>
			<Card href={resolve('/docs')}>docs{#snippet icon()}{site.glyph}{/snippet}</Card>
		</section>
		<section>
			<DocsFooter repo_url={site.repo_url} root_url={FUZ_DEV_URL}>
				{#snippet logo_header()}<a href={resolve('/about')} class="mb_xs">about</a>{/snippet}
			</DocsFooter>
		</section>
	</div>
</main>

<style>
	main {
		margin-bottom: var(--space_xl5);
	}
	section {
		margin-top: var(--space_xl3);
		display: flex;
		flex-direction: column;
		align-items: center;
	}
</style>
