// a caught refusal's warning prints after the run's error, which the program wrote
try {
	await import('ext:core/ops');
} catch {}
throw new Error('x' + '\x1b[1A\x1b[2K'.repeat(3));
