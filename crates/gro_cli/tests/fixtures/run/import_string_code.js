// the imports of a module `import()` loads resolve with no JS frame on the stack
await import('./import_static.js');
// an `import()` in code compiled from a string is refused, whatever it names
for (const compile of [
	(source) => (0, eval)(source),
	(source) => new Function(`return ${source}`)()
]) {
	try {
		await compile("import('./sibling.js')");
		console.log('imported');
	} catch (error) {
		console.log(error instanceof TypeError, error.message);
	}
}
