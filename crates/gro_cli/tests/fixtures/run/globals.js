// what user code can see of the host's internals
const g = /** @type {any} */ (globalThis);
console.log(
	JSON.stringify({
		deno: typeof g.Deno,
		bootstrap: typeof g.__bootstrap,
		deno_core: typeof g.Deno?.core,
		own: Object.getOwnPropertyNames(globalThis).filter((name) => /deno|bootstrap/i.test(name)),
		enumerable: Object.keys(globalThis),
		fetch: typeof g.fetch,
		report_error: typeof g.reportError,
		web: [
			typeof URL,
			typeof TextEncoder,
			typeof setTimeout,
			typeof structuredClone,
			typeof AbortController,
			typeof ReadableStream,
			typeof performance,
			typeof queueMicrotask
		]
	})
);
