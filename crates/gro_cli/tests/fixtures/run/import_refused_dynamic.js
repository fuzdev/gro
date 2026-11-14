// refusals reach `import()` as catchable errors, the same path as static imports, and probing
// for what isn't there, for a feature, or with `import.meta.resolve` doesn't fail the run
for (const specifier of ['node:fs', 'svelte', './sibling', './enum.ts']) {
	try {
		await import(specifier);
	} catch (error) {
		console.log(error instanceof TypeError, JSON.stringify(String(error)));
	}
}
for (const attributes of [{ lazy: 'true' }, { type: 'json' }]) {
	try {
		await import('./sibling.js', { with: attributes });
	} catch (error) {
		console.log(error instanceof TypeError, JSON.stringify(String(error)));
	}
}
for (const specifier of ['@fuzdev/gro/args.js', 'ext:core/ops']) {
	try {
		import.meta.resolve(specifier);
	} catch (error) {
		console.log(error instanceof TypeError, JSON.stringify(String(error)));
	}
}
