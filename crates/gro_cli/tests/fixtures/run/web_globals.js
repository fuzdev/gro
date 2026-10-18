// touches each installed web global far enough to run its lazily loaded internals
const results = [];
const check = (name, value) => results.push(`${name}=${value}`);

check('url', new URL('../b?q=1#h', 'file:///a/c/d').href);
check('search', new URLSearchParams({ a: '1', b: 'x y' }).toString());
check(
	'pattern',
	new URLPattern({ pathname: '/users/:id' }).exec('https://x.test/users/7')?.pathname.groups.id
);
check('encoding', new TextDecoder().decode(new TextEncoder().encode('café')).length);
check('base64', atob(btoa('gro')));
check('clone', structuredClone(new Map([['k', [1, 2]]])).get('k').length);
check('dom_exception', new DOMException('m', 'AbortError').name);
const target = new EventTarget();
let heard = '';
target.addEventListener('ping', (event) => (heard = /** @type {CustomEvent} */ (event).detail));
target.dispatchEvent(new CustomEvent('ping', { detail: 'pong' }));
check('event', heard);
const controller = new AbortController();
controller.abort();
check('abort', controller.signal.aborted);
check('blob', await new Blob(['ab', 'c'], { type: 'text/plain' }).text());
check('file', new File(['x'], 'f.txt').name);
const piped = new Blob(['streamed']).stream().pipeThrough(new TextDecoderStream());
let streamed = '';
for await (const chunk of piped) streamed += chunk;
check('stream', streamed);
const compressed = new Blob(['zip'.repeat(50)]).stream().pipeThrough(new CompressionStream('gzip'));
const round = compressed.pipeThrough(new DecompressionStream('gzip'));
check('compression', (await read_text(round)).length);
check('performance', performance.now() >= 0);
console.log(results.join('\n'));

/** Reads a byte stream to a string. */
async function read_text(/** @type {ReadableStream<Uint8Array>} */ stream) {
	let text = '';
	const decoder = new TextDecoder();
	for await (const chunk of stream) text += decoder.decode(chunk, { stream: true });
	return text + decoder.decode();
}
