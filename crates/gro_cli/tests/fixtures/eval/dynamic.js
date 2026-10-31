// every import() is refused, whatever it names, and the refusal is catchable
const caught = [];
for (const specifier of ['./value.js', 'node:fs']) {
	try {
		await import(specifier);
	} catch (error) {
		caught.push({ type_error: error instanceof TypeError, message: String(error.message) });
	}
}

export default caught;
