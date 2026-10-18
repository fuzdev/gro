// refusals reach `import()` as catchable errors, the same path as static imports
for (const specifier of ['node:fs', 'svelte', './sibling', './enum.ts']) {
	try {
		await import(specifier);
	} catch (error) {
		console.log(error instanceof TypeError, JSON.stringify(String(error)));
	}
}
