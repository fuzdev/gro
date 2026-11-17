// a cause whose message names a frame, as a strip's refusal does, breaks there
try {
	await import('./enum.ts');
} catch (error) {
	throw new Error('wrapped', { cause: error });
}
