// every web global: its constructor and a method or two, so the ops and classes the
// globals' code calls are called through it (`console.trace`'s is in `console_trace.js`)
const results = [];
const check = (/** @type {string} */ name, /** @type {unknown} */ value) =>
	results.push(`${name}=${value}`);
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

const url = new URL('https://user@example.test:8080/a/b?x=1#frag');
url.searchParams.append('y', '2');
url.hash = '';
check('URL', `${url.href} ${URL.canParse('nope')} ${URL.parse('/p', 'https://x.test')?.href}`);
const params = new URLSearchParams('b=2&a=1');
params.sort();
check('URLSearchParams', `${params} ${params.get('a')} ${params.has('c')}`);
const pattern = new URLPattern('https://*.example.test/:kind/*');
check('URLPattern', `${pattern.test('https://a.example.test/img/x.png')} ${pattern.pathname}`);
const object_url = URL.createObjectURL(new Blob(['o']));
URL.revokeObjectURL(object_url);
check('object_url', object_url.startsWith('blob:'));

const exception = new DOMException('m', 'DataCloneError');
check('DOMException', `${exception.name} ${exception.code} ${exception instanceof Error}`);
check('atob', `${atob(btoa('gro'))} ${thrown(() => atob('!'))}`);

const target = new EventTarget();
const heard = [];
target.addEventListener('e', (event) => heard.push(event.type), { once: true });
target.dispatchEvent(new Event('e', { cancelable: true }));
target.dispatchEvent(new Event('e'));
target.addEventListener('c', (event) => heard.push(/** @type {CustomEvent} */ (event).detail));
target.dispatchEvent(new CustomEvent('c', { detail: 'detail' }));
check('events', heard.join(','));
check('ErrorEvent', new ErrorEvent('error', { message: 'm', lineno: 3 }).lineno);
const rejection = new PromiseRejectionEvent('unhandledrejection', {
	promise: Promise.resolve(),
	reason: 'why'
});
check('PromiseRejectionEvent', rejection.reason);

const buffer = new ArrayBuffer(8);
const moved = structuredClone(buffer, { transfer: [buffer] });
const cloned = structuredClone({ date: new Date(0), map: new Map([['k', new Set([1n])]]) });
check(
	'structuredClone',
	`${moved.byteLength} ${buffer.byteLength} ${cloned.date.getTime()} ${cloned.map.get('k')?.has(1n)} ` +
		thrown(() => structuredClone(() => {}))
);

check('setTimeout', await new Promise((resolve) => setTimeout(resolve, 1, 'fired')));
let ticks = 0;
await new Promise((resolve) => {
	const id = setInterval(() => {
		if (++ticks === 2) {
			clearInterval(id);
			resolve(undefined);
		}
	}, 1);
});
clearTimeout(setTimeout(() => check('cleared', 'ran'), 1));
check('setInterval', ticks);
check('queueMicrotask', await new Promise((resolve) => queueMicrotask(() => resolve('ran'))));

const controller = new AbortController();
const any = AbortSignal.any([controller.signal]);
controller.abort('why');
check(
	'AbortController',
	`${controller.signal.reason} ${any.aborted} ${AbortSignal.abort().aborted}`
);
check(
	'AbortSignal.timeout',
	await new Promise((resolve) => {
		const signal = AbortSignal.timeout(1);
		signal.addEventListener('abort', () => resolve(signal.reason.name));
	})
);

const readable = new ReadableStream(
	{
		start(stream) {
			stream.enqueue('a');
			stream.enqueue('b');
			stream.close();
		}
	},
	new CountQueuingStrategy({ highWaterMark: 2 })
);
const upper = new TransformStream({
	transform(chunk, stream) {
		stream.enqueue(chunk.toUpperCase());
	}
});
const written = [];
await readable.pipeThrough(upper).pipeTo(
	new WritableStream({
		write(chunk) {
			written.push(chunk);
		}
	})
);
check('streams', written.join(''));
const bytes = new ReadableStream({
	type: 'bytes',
	start(stream) {
		stream.enqueue(new Uint8Array([1, 2, 3]));
		stream.close();
	}
});
const byob = bytes.getReader({ mode: 'byob' });
const read = await byob.read(new Uint8Array(8));
check('byob', `${read.value?.length} ${byob instanceof ReadableStreamBYOBReader}`);
const [left, right] = new ReadableStream({
	start(stream) {
		stream.enqueue(1);
		stream.close();
	}
}).tee();
const reader = left.getReader();
check(
	'tee',
	`${(await reader.read()).value} ${(await right.getReader().read()).value} ` +
		(reader instanceof ReadableStreamDefaultReader)
);
const writer = new WritableStream().getWriter();
await writer.write('w');
await writer.close();
check('writer', writer instanceof WritableStreamDefaultWriter);
check(
	'ByteLengthQueuingStrategy',
	new ByteLengthQueuingStrategy({ highWaterMark: 16 }).highWaterMark
);
check(
	'stream_classes',
	[
		ReadableStreamBYOBRequest,
		ReadableByteStreamController,
		ReadableStreamDefaultController,
		TransformStreamDefaultController,
		WritableStreamDefaultController
	]
		.map((Class) => thrown(() => new /** @type {any} */ (Class)()))
		.join(',')
);

const encoded = new TextEncoder().encode('café');
const into = new TextEncoder().encodeInto('éé', new Uint8Array(3));
check('TextEncoder', `${encoded.length} ${into.read} ${into.written}`);
const streaming = new TextDecoder();
check(
	'TextDecoder',
	[
		new TextDecoder().decode(encoded),
		streaming.decode(encoded.subarray(0, 4), { stream: true }) +
			streaming.decode(encoded.subarray(4)),
		new TextDecoder('utf-16le').decode(new Uint8Array([0x67, 0, 0x72, 0])),
		new TextDecoder('windows-1252').decode(new Uint8Array([0xe9])),
		thrown(() => new TextDecoder('utf-8', { fatal: true }).decode(new Uint8Array([0xff]))),
		thrown(() => new TextDecoder('nope'))
	].join(' ')
);
const encoder_stream = new TextEncoderStream();
const encoder_writer = encoder_stream.writable.getWriter();
void encoder_writer.write('st').then(() => encoder_writer.close());
check('text_streams', await read_text(encoder_stream.readable));

const blob = new Blob(['ab', new Uint8Array([99])], { type: 'text/plain' });
check(
	'Blob',
	`${blob.size} ${blob.type} ${await blob.slice(1).text()} ${(await blob.arrayBuffer()).byteLength} ` +
		`${(await blob.bytes()).length} ${await read_text(blob.stream())}`
);
const file = new File([blob, 'd'], 'f.txt', { lastModified: 5 });
check('File', `${file.name} ${file.lastModified} ${await file.text()} ${file instanceof Blob}`);
const copied = structuredClone(blob);
check('Blob_clone', `${copied instanceof Blob} ${copied !== blob} ${await copied.text()}`);

for (const format of /** @type {const} */ (['gzip', 'deflate', 'deflate-raw'])) {
	const round = new Blob(['zip'.repeat(50)])
		.stream()
		.pipeThrough(new CompressionStream(format))
		.pipeThrough(new DecompressionStream(format));
	check(format, (await read_text(round)).length);
}
check(
	'CompressionStream',
	thrown(() => new CompressionStream(/** @type {any} */ ('nope')))
);

performance.mark('a');
performance.measure('m', 'a');
check(
	'performance',
	`${performance.getEntriesByName('m').length} ${performance.now() >= 0} ` +
		`${performance.timeOrigin > 0} ${typeof performance.toJSON().timeOrigin}`
);

console.log(results.join('\n'));
// the console's own methods, each printing what it's given
console.log('%s %d %i %f %o %O %c%%', 'text', 1.5, 1.5, 1.5, { a: [1] }, { b: 2 }, 'color: red');
console.log(new Map([[1, { a: 1 }]]), new Set(['s']), [1, , 3], 10n, Symbol('s'), null, undefined);
console.log(new URL('https://x.test/p?q'), new Uint8Array(2), new Error('shown').message, -0);
console.info('info');
console.debug('debug');
console.dir({ nested: { deep: { deeper: {} } } }, { depth: 0 });
console.dirxml('dirxml');
console.table([{ a: 1, b: 'x' }]);
console.group('group');
console.log('inside');
console.groupEnd();
console.count();
console.count('named');
console.countReset();
console.count();
console.assert(true, 'unseen');
console.time('timed');
