// an uncaught probe is the run's error, and listed after it with the caught refusal
try {
	await import('ext:core/ops');
} catch {}
await import('@fuzdev/gro/process.js');
