<script lang="ts" module>
	export const a = 'ok';

	/* eslint-disable svelte/no-useless-mustaches */
</script>

<script lang="ts">
	const {
		b = 'ok'
	}: {
		b: string;
	} = $props();

	// `
	// ``
	// ```
	`backticks`;
	('`');
	('``');
	('```');
	`\``;
	`\`\``;
	`\`\`\``;
</script>

SomeTestSvelte.svelte contents

{a}

{b}

` `` ```
<!-- ` -->
<!-- `` -->
<!-- ``` -->
{`backticks`}
{'`'}
{'``'}
{'```'}
{`\``}
{`\`\``}
{`\`\`\``}
