// every web global eval has: its constructor and a method or two, so the ops and
// classes the globals' code calls are called through it
/** What calling `f` throws, by name. */
const thrown = (/** @type {() => unknown} */ f) => {
	try {
		f();
		return 'nothing';
	} catch (error) {
		return /** @type {Error} */ (error).name;
	}
};
/** Reads a byte stream to a string. */
const read_text = async (/** @type {ReadableStream<Uint8Array>} */ stream) => {
	let text = '';
	for await (const chunk of stream.pipeThrough(new TextDecoderStream())) text += chunk;
	return text;
};
/** A byte stream of `text`. */
const byte_stream = (/** @type {string} */ text) =>
	new ReadableStream({
		start(stream) {
			stream.enqueue(new TextEncoder().encode(text));
			stream.close();
		}
	});

const url = new URL('https://user@example.test:8080/a/b?x=1#frag');
url.searchParams.append('y', '2');
url.hash = '';
const params = new URLSearchParams('b=2&a=1');
params.sort();
const pattern = new URLPattern('https://*.example.test/:kind/*');

const exception = new DOMException('m', 'DataCloneError');

const target = new EventTarget();
/** @type {Array<string>} */
const heard = [];
target.addEventListener('e', (event) => heard.push(event.type), { once: true });
target.dispatchEvent(new Event('e', { cancelable: true }));
target.dispatchEvent(new Event('e'));
target.addEventListener('c', (event) => heard.push(/** @type {CustomEvent} */ (event).detail));
target.dispatchEvent(new CustomEvent('c', { detail: 'detail' }));
const rejection = new PromiseRejectionEvent('unhandledrejection', {
	promise: Promise.resolve(),
	reason: 'why'
});

const buffer = new ArrayBuffer(8);
const moved = structuredClone(buffer, { transfer: [buffer] });
const cloned = structuredClone({ map: new Map([['k', new Set([1n])]]) });

const controller = new AbortController();
const any = AbortSignal.any([controller.signal]);
controller.abort('why');

const upper = new TransformStream({
	transform(chunk, stream) {
		stream.enqueue(chunk.toUpperCase());
	}
});
/** @type {Array<string>} */
const written = [];
await new ReadableStream(
	{
		start(stream) {
			stream.enqueue('a');
			stream.enqueue('b');
			stream.close();
		}
	},
	new CountQueuingStrategy({ highWaterMark: 2 })
)
	.pipeThrough(upper)
	.pipeTo(
		new WritableStream({
			write(chunk) {
				written.push(chunk);
			}
		})
	);
const byob = new ReadableStream({
	type: 'bytes',
	start(stream) {
		stream.enqueue(new Uint8Array([1, 2, 3]));
		stream.close();
	}
}).getReader({ mode: 'byob' });
const read = await byob.read(new Uint8Array(8));
const writer = new WritableStream().getWriter();
await writer.write('w');
await writer.close();

const encoded = new TextEncoder().encode('café');
const into = new TextEncoder().encodeInto('éé', new Uint8Array(3));
const encoder_stream = new TextEncoderStream();
const encoder_writer = encoder_stream.writable.getWriter();
void encoder_writer.write('st').then(() => encoder_writer.close());

/** @type {Record<string, number>} */
const compression = {};
for (const format of /** @type {const} */ (['gzip', 'deflate', 'deflate-raw'])) {
	const round = byte_stream('zip'.repeat(50))
		.pipeThrough(new CompressionStream(format))
		.pipeThrough(new DecompressionStream(format));
	compression[format] = (await read_text(round)).length;
}

console.log('%s %d %o %c%%', 'text', 1.5, { a: [1] }, 'color: red');
console.log(
	new Map([[1, { a: 1 }]]),
	new Set(['s']),
	[1, , 3],
	10n,
	Symbol('s'),
	new URL('https://x.test/p?q')
);
console.table([{ a: 1, b: 'x' }]);
console.group('group');
console.dir({ nested: { deep: {} } }, { depth: 0 });
console.groupEnd();
console.count();
console.assert(false, 'asserted');

export default {
	url: `${url.href} ${URL.canParse('nope')} ${URL.parse('/p', 'https://x.test')?.href}`,
	search_params: `${params} ${params.get('a')} ${params.has('c')}`,
	url_pattern: `${pattern.test('https://a.example.test/img/x.png')} ${pattern.pathname}`,
	object_urls: `${typeof (/** @type {any} */ (URL).createObjectURL)} ${typeof Blob} ${typeof File}`,
	dom_exception: `${exception.name} ${exception.code} ${exception instanceof Error}`,
	base64: `${atob(btoa('gro'))} ${thrown(() => atob('!'))}`,
	events: heard.join(','),
	error_event: new ErrorEvent('error', { message: 'm', lineno: 3 }).lineno,
	rejection_event: rejection.reason,
	structured_clone:
		`${moved.byteLength} ${buffer.byteLength} ${cloned.map.get('k')?.has(1n)} ` +
		thrown(() => structuredClone(() => {})),
	timers: `${typeof setTimeout} ${typeof clearInterval} ${thrown(() => AbortSignal.timeout(1))}`,
	microtask: await new Promise((resolve) => queueMicrotask(() => resolve('ran'))),
	abort: `${controller.signal.reason} ${any.aborted} ${AbortSignal.abort().aborted}`,
	streams: written.join(''),
	byob: `${read.value?.length} ${byob instanceof ReadableStreamBYOBReader}`,
	writer: writer instanceof WritableStreamDefaultWriter,
	strategy: new ByteLengthQueuingStrategy({ highWaterMark: 16 }).highWaterMark,
	stream_classes: [
		ReadableStreamBYOBRequest,
		ReadableByteStreamController,
		ReadableStreamDefaultController,
		ReadableStreamDefaultReader,
		TransformStreamDefaultController,
		WritableStreamDefaultController
	]
		.map((Class) => thrown(() => new /** @type {any} */ (Class)()))
		.join(','),
	text_encoder: `${encoded.length} ${into.read} ${into.written}`,
	text_decoder: [
		new TextDecoder().decode(encoded),
		new TextDecoder('utf-16le').decode(new Uint8Array([0x67, 0, 0x72, 0])),
		new TextDecoder('windows-1252').decode(new Uint8Array([0xe9])),
		thrown(() => new TextDecoder('utf-8', { fatal: true }).decode(new Uint8Array([0xff]))),
		thrown(() => new TextDecoder('nope'))
	].join(' '),
	text_streams: await read_text(encoder_stream.readable),
	compression,
	compression_format: thrown(() => new CompressionStream(/** @type {any} */ ('nope'))),
	performance: `${typeof performance.getEntries} ${thrown(() => performance.now())} ${thrown(() => performance.mark('a'))}`
};
