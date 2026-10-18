// gro's bootstrap: installs the pure web globals deno_web provides, then
// removes the host's internals from the global object.
//
// Pure globals only: nothing here reaches the network, the filesystem, the
// environment, or another process. Authority globals (`fetch` and the like)
// will install only under a grant. The set is the part of the WinterTC
// minimum common API that deno_web carries (`fetch` and `crypto` live in other
// crates, not linked). `reportError`, `self`, and the global object as an
// `EventTarget` wait for a consumer: deno_web's `reportError` dispatches on a
// global event target this bootstrap doesn't set up.
//
// This file must stay 7-bit ASCII: the build fails otherwise.

import { core } from 'ext:core/mod.js';
import { op_gro_color } from 'ext:core/ops';

const load = core.loadExtScript;

const console_ns = load('ext:deno_web/01_console.js');
const url_ns = load('ext:deno_web/00_url.js');
const url_pattern_ns = load('ext:deno_web/01_urlpattern.js');
const dom_exception_ns = load('ext:deno_web/01_dom_exception.js');
const event_ns = load('ext:deno_web/02_event.js');
const structured_clone_ns = load('ext:deno_web/02_structured_clone.js');
const timers_ns = load('ext:deno_web/02_timers.js');
const abort_signal_ns = load('ext:deno_web/03_abort_signal.js');
const base64_ns = load('ext:deno_web/05_base64.js');
const streams_ns = load('ext:deno_web/06_streams.js');
const text_encoding_ns = load('ext:deno_web/08_text_encoding.js');
const file_ns = load('ext:deno_web/09_file.js');
const compression_ns = load('ext:deno_web/14_compression.js');
const performance_ns = load('ext:deno_web/15_performance.js');

// colored output only to a terminal, and never with NO_COLOR or CLAUDECODE set
// non-empty (the host's `use_color` decides, once per process); asked per write,
// not at bootstrap, so the snapshot never carries the answer
console_ns.setNoColorFns(
	() => !op_gro_color(false),
	() => !op_gro_color(true)
);

/** @type {Record<string, unknown>} */
const globals = {
	console: new console_ns.Console((/** @type {string} */ message, /** @type {number} */ level) =>
		// `warn` and `error` (levels 2 and 3) go to stderr
		core.print(message, level > 1)
	),

	URL: url_ns.URL,
	URLSearchParams: url_ns.URLSearchParams,
	URLPattern: url_pattern_ns.URLPattern,

	DOMException: dom_exception_ns.DOMException,
	Event: event_ns.Event,
	EventTarget: event_ns.EventTarget,
	CustomEvent: event_ns.CustomEvent,
	ErrorEvent: event_ns.ErrorEvent,
	PromiseRejectionEvent: event_ns.PromiseRejectionEvent,

	structuredClone: structured_clone_ns.structuredClone,

	setTimeout: timers_ns.setTimeout,
	clearTimeout: timers_ns.clearTimeout,
	setInterval: timers_ns.setInterval,
	clearInterval: timers_ns.clearInterval,

	AbortController: abort_signal_ns.AbortController,
	AbortSignal: abort_signal_ns.AbortSignal,

	atob: base64_ns.atob,
	btoa: base64_ns.btoa,

	ReadableStream: streams_ns.ReadableStream,
	ReadableStreamDefaultReader: streams_ns.ReadableStreamDefaultReader,
	ReadableStreamBYOBReader: streams_ns.ReadableStreamBYOBReader,
	ReadableStreamBYOBRequest: streams_ns.ReadableStreamBYOBRequest,
	ReadableByteStreamController: streams_ns.ReadableByteStreamController,
	ReadableStreamDefaultController: streams_ns.ReadableStreamDefaultController,
	TransformStream: streams_ns.TransformStream,
	TransformStreamDefaultController: streams_ns.TransformStreamDefaultController,
	WritableStream: streams_ns.WritableStream,
	WritableStreamDefaultWriter: streams_ns.WritableStreamDefaultWriter,
	WritableStreamDefaultController: streams_ns.WritableStreamDefaultController,
	ByteLengthQueuingStrategy: streams_ns.ByteLengthQueuingStrategy,
	CountQueuingStrategy: streams_ns.CountQueuingStrategy,

	TextEncoder: text_encoding_ns.TextEncoder,
	TextDecoder: text_encoding_ns.TextDecoder,
	TextEncoderStream: text_encoding_ns.TextEncoderStream,
	TextDecoderStream: text_encoding_ns.TextDecoderStream,

	Blob: file_ns.Blob,
	File: file_ns.File,

	CompressionStream: compression_ns.CompressionStream,
	DecompressionStream: compression_ns.DecompressionStream,

	performance: performance_ns.performance,

	// deno_core's own, redefined here so it has the shape of the rest whether
	// the runtime starts from a snapshot or not (the snapshot build leaves it
	// enumerable)
	queueMicrotask: /** @type {any} */ (globalThis).queueMicrotask
};

// the web's shape for interface objects and operations: writable,
// configurable, not enumerable
for (const [name, value] of Object.entries(globals)) {
	Object.defineProperty(globalThis, name, {
		value,
		writable: true,
		enumerable: false,
		configurable: true
	});
}

// the steps that must happen in each process, never in a snapshot: the
// runtime calls this once it has started, from a snapshot or not. deno_core
// reads `Deno.core` while it starts from a snapshot, so the internals stay
// until then.
/** @type {any} */ (globalThis).__bootstrap.gro_start = () => {
	performance_ns.setTimeOrigin();
	// deno_core's internals: `Deno.core` reaches every op, and gro has no
	// `Deno` namespace. Hygiene, not a boundary: no grant check relies on this.
	delete (/** @type {any} */ (globalThis).__bootstrap);
	delete (/** @type {any} */ (globalThis).Deno);
};
