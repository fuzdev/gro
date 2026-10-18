<script lang="ts">
	import TomeContent from '@fuzdev/fuz_ui/TomeContent.svelte';
	import { tome_get_by_slug } from '@fuzdev/fuz_ui/tome.ts';

	const TOME_SLUG = 'introduction';

	const tome = tome_get_by_slug(TOME_SLUG);
</script>

<TomeContent {tome}>
	<section>
		<p>
			Gro is a task runner and toolkit extending SvelteKit. It's part of the
			<a href="https://www.fuz.dev/">Fuz stack</a>.
		</p>
		<p>
			Gro is also becoming its own JS runtime: a security-first Rust binary on V8 with granular
			control over what code can reach, built for code you don't fully trust, like dependencies and
			agents. It's experimental; the roadmap, security model, and design are in
			<a href="https://github.com/fuzdev/gro/blob/main/src/docs/runtime.md">the runtime docs</a>.
		</p>
		<p>
			This site hosts only the API reference. The guides and full docs are in markdown on the
			<a href="https://github.com/fuzdev/gro">GitHub repo</a>.
		</p>
	</section>
</TomeContent>
