// refusals reach `import()` as catchable errors, the same path as static imports
for (const specifier of ['node:fs', 'svelte', './typed.ts', './sibling']) {
	try {
		await import(specifier);
	} catch (error) {
		console.log(error instanceof TypeError, String(error));
	}
}
