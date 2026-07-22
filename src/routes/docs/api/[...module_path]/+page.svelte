<script lang="ts">
	import ApiModule from '@fuzdev/fuz_ui/ApiModule.svelte';

	const { params } = $props();

	const module_path = $derived(params.module_path ?? '');
</script>

<ApiModule {module_path} />
