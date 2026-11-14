// an import() with attributes is refused as every import() is, before its attributes are read
const caught = [];
for (const [specifier, attributes] of [
	['./value.js', { x: 'y' }],
	['@fuzdev/gro/process.ts', { type: 'json' }]
]) {
	try {
		await import(specifier, { with: attributes });
	} catch (error) {
		caught.push({ type_error: error instanceof TypeError, message: String(error.message) });
	}
}

export default caught;
