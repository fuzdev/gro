// an uncaught refusal is the run's error, printed once, and the caught one is listed after it
try {
	await import('ext:core/ops');
} catch {}
await import('@fuzdev/gro/args.js');
