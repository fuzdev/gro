/**
 * The net host module: the network as an import rather than a global.
 *
 * `net.fetch` is an own frozen closure, so `const {fetch} = net` works and it can be handed
 * to anything that takes an injected `fetch`. There's no bare `fetch` export: the only way
 * to the network is through a handle, and `net.narrow({hosts})` makes a narrower one to hand down.
 *
 * A host is `hostname`, any port, or `hostname:port`, normalized through `URL`, one trailing
 * dot dropped, so a grant is compared as a request's URL will be sent. Redirects are the
 * escape a host check has to close, so a handle follows them itself: a `Location` is
 * checked before anything is sent to it, and a refused host rejects with `not_granted` and
 * no request sent. A caller's `redirect: 'manual'` or `'error'` passes through unchanged.
 *
 * Handles and their prototypes are frozen, null-prototype, and constructing one takes a key
 * only this module holds. A handle's hosts live in private fields, so `narrow` called on any other
 * object throws `TypeError` and `fetch`, a closure, ignores its receiver.
 *
 * This is the Node implementation, which runs on the Node Gro. Its gaps against the
 * runtime's builtin, which it documents rather than the API weakening to match:
 *
 * - The hostname is checked, not the address it connects to, so a granted name resolving to a
 *   private range, or rebinding between requests, isn't refused.
 * - The globals stay: any module can call the global `fetch` or import `node:http`,
 *   so a handle bounds only code that uses it, a review signal, not a boundary.
 * - The root handle allows every host, where the runtime's holds only the accepted ones.
 * - Following relies on Node exposing a `manual` response's status and `Location`,
 *   where the Fetch standard filters them behind an `opaqueredirect`.
 * - The dispatcher every send names delegates to undici's versioned global dispatcher, read
 *   each send, so a proxy from `setGlobalDispatcher` keeps working, and so can a dispatcher
 *   a module installs there to reroute every send. A missing one rejects rather than send past it.
 * - Beside native follow, following differs in five ways:
 *   a response after a cross-origin hop has `type` `basic`, not `cors`, nor is a `no-cors`
 *   one opaque, as each hop is its own request;
 *   a `Request` with a stream body is buffered and replayed, as its body's source isn't
 *   observable, buffering whether or not a redirect comes, so a large upload streams through
 *   `init`, where a stream is refused as native follow refuses one;
 *   a request with `integrity` that's redirected rejects unless every hop's body matches,
 *   as the integrity check runs on each hop rather than only the last;
 *   a non-HTTP(S) `Location` is `not_granted` rather than a `TypeError`;
 *   and a redirect's own `Referrer-Policy` isn't applied to the next hop.
 * - Intrinsics stay pollutable. `fetch`, `Request`, `Response`, `Headers`, `URL`, and `Object`'s
 *   functions are copied at load, so a `fetch` replaced before this module loads is the one
 *   called, but code that patches their prototypes after this module loads runs inside these ops,
 *   and a `Response` is a real one, so awaiting it adopts a polluted `then`.
 * - `Object.prototype` stays reachable. undici's init dictionary reads a key the init doesn't own
 *   through it, and undici's `Client` and `net.connect` read their connection options through it,
 *   so a polluted `Object.prototype` can change the connection, including its destination, through
 *   options like `socketPath` and `lookup`. That is part of this module being advisory, and no list
 *   of polluted keys could be complete. Every send names `redirect` and the dispatcher,
 *   so an ordinary option or a `Request` can't reroute or follow.
 *
 * @module
 */

import * as node_buffer from 'node:buffer';

import { create_record, read_own } from './internal/records.ts';
import { NetError } from './net_error.ts';

const {
	fetch: native_fetch,
	Request: RequestClass,
	Headers: HeadersClass,
	URL: URLClass
} = globalThis;
const { Buffer: NodeBuffer } = node_buffer;
const { clone: response_clone } = Response.prototype;
const { versions } = globalThis.process;
const { isArray } = Array;
const { defineProperty, freeze, setPrototypeOf } = Object;

// the key only this module holds, so a handle's `constructor` can't make another handle
const HANDLE_KEY = Symbol('net handle');

const check_handle_key = (key: symbol): void => {
	if (key !== HANDLE_KEY) throw new TypeError('net handles come from `net`, not a constructor');
};

/**
 * Hostnames allowing any port, and `hostname:port` entries allowing one.
 * Null on the root handle, which allows every host.
 */
type Allowed = Readonly<Record<string, true | undefined>>;

const MAX_REDIRECTS = 20;

// the global key this Node's built-in `fetch` reads, one per undici major
const GLOBAL_DISPATCHER = Symbol.for(
	`undici.globalDispatcher.${Number(versions.undici!.split('.')[0]) >= 8 ? 2 : 1}`
);

// the dispatcher every send names, replacing any `dispatcher` an init or `Request` carried
const HOST_DISPATCHER = freeze(
	create_record({
		dispatch: (options: unknown, handler: unknown): boolean => {
			const dispatcher = (globalThis as any)[GLOBAL_DISPATCHER];
			if (typeof dispatcher?.dispatch !== 'function') {
				throw new TypeError(
					`undici's global dispatcher is missing at ${String(GLOBAL_DISPATCHER)}`
				);
			}
			return dispatcher.dispatch(options, handler);
		}
	})
);

// removed from a request that changes origin
const CROSS_ORIGIN_HEADERS = ['authorization', 'cookie', 'proxy-authorization'];

// removed from a request that loses its body
const BODY_HEADERS = ['content-encoding', 'content-language', 'content-location', 'content-type'];

/**
 * The options of `narrow`.
 */
export interface NetNarrowOptions {
	/**
	 * The hosts the new handle reaches, each `hostname` (any port) or `hostname:port`.
	 * An empty list reaches nothing.
	 */
	hosts: ReadonlyArray<string>;
}

/**
 * The network, reached through a handle bound to its hosts.
 */
export interface Net {
	/**
	 * `fetch`, bound to this handle's hosts: an own frozen closure, so it destructures and
	 * hands down as a bare function, and it's assignable to `typeof globalThis.fetch`.
	 * Redirects are followed by this handle, each `Location` checked before anything
	 * is sent to it. A caller's `redirect: 'manual'` or `'error'` passes through unchanged,
	 * so a `manual` caller sees the response and decides.
	 * @throws `NetError` - `not_granted`, a host outside this handle's set, or a scheme
	 * other than `http:` and `https:`, on the first request or a redirect
	 * @throws `TypeError` - a network failure, an invalid `Location`, more than 20 redirects,
	 * or a redirect that can't replay a stream body
	 */
	readonly fetch: (input: string | URL | Request, init?: RequestInit) => Promise<Response>;
	/**
	 * A handle limited to `hosts`, as `env.narrow` limits names. It can only shrink:
	 * a narrower one holds `hostname:port` beneath a `hostname` it's allowed, never the reverse.
	 * @throws `TypeError` - for a host that isn't exact: `*`, a scheme, a path, or a value `URL` refuses
	 * @throws `NetError` - `not_granted`, a host this handle doesn't allow
	 */
	narrow(options: NetNarrowOptions): Net;
}

const refuse_host = (value: unknown): never => {
	throw new TypeError(`a host is \`hostname\` or \`hostname:port\`: ${String(value)}`);
};

/**
 * A hostname, as it's compared: one trailing dot dropped, the name `example.com.` and
 * `example.com` both spell, so a grant can't be missed or dodged by a dot.
 */
const to_hostname = (url: URL): string => {
	const { hostname } = url;
	return hostname.endsWith('.') ? hostname.slice(0, -1) : hostname;
};

/**
 * The normalized hostname and table key for a host given to `narrow`.
 * `URL` is the parser, so a grant spells a name as a request's URL does (case, IDNA, IP forms);
 * it's refused first for anything `URL` would silently drop or read as more than a host.
 */
const parse_host = (value: unknown): { hostname: string; key: string } => {
	if (typeof value !== 'string') throw new TypeError('a host must be a string');
	if (value.includes('*')) throw new TypeError(`a host is exact, never a glob: ${value}`);
	if (!value.isWellFormed() || /[\p{Cc}\s/\\?#@]/u.test(value) || value.endsWith(':')) {
		refuse_host(value);
	}
	let http_url: URL;
	let https_url: URL;
	try {
		http_url = new URLClass(`http://${value}`);
		https_url = new URLClass(`https://${value}`);
	} catch {
		return refuse_host(value);
	}
	const hostname = to_hostname(http_url);
	if (hostname === '') refuse_host(value);
	// `URL` drops a scheme's default port, so `:80` shows only under `https` and `:443` only under `http`
	const port = http_url.port || https_url.port;
	return { hostname, key: port === '' ? hostname : `${hostname}:${port}` };
};

/**
 * Whether `url` is http(s) and its host is allowed, else `not_granted`.
 */
const check_url = (allowed: Allowed | null, url: URL): void => {
	const { protocol, port } = url;
	if (protocol !== 'http:' && protocol !== 'https:') {
		throw new NetError('not_granted', protocol, `not_granted: ${protocol} isn't http or https`);
	}
	if (allowed === null) return;
	const hostname = to_hostname(url);
	const effective_port = port === '' ? (protocol === 'https:' ? '443' : '80') : port;
	const host = `${hostname}:${effective_port}`;
	if (allowed[hostname] !== true && allowed[host] !== true) {
		throw new NetError('not_granted', host, `not_granted: ${host} isn't in this net's hosts`);
	}
};

/**
 * The URL a redirect response points at, resolved against the response's own URL.
 * Null for a response that isn't a redirect to follow, as native follow returns it as-is.
 * Non-ASCII in a `Location` arrives as latin1 bytes, decoded as UTF-8 as native follow does.
 * @throws `TypeError` - for a `Location` that isn't a URL
 */
const to_redirect_url = (response: Response, url: URL): URL | null => {
	const { status } = response;
	if (status !== 301 && status !== 302 && status !== 303 && status !== 307 && status !== 308) {
		return null;
	}
	let location = response.headers.get('location');
	if (location === null) return null;
	// `[^\x21-\x7e]`: any byte outside printable ASCII means the header was raw bytes
	if (/[^\x21-\x7e]/.test(location)) {
		location = NodeBuffer.from(location, 'latin1').toString('utf8');
	}
	try {
		return new URLClass(location, url);
	} catch {
		// no `cause`, which would hold the URL, token and all
		throw new TypeError('a redirect has an invalid `Location`');
	}
};

const ignore_error = (): void => {
	// nothing reads a discarded body, so its cancel has no one to report to
};

/**
 * Marks a followed response as redirected, as native follow would, on it and any clone of it.
 * `url` needs no change: the last hop's own response is at the final URL.
 */
const mark_redirected = (response: Response): Response => {
	defineProperty(response, 'redirected', { value: true });
	defineProperty(response, 'clone', {
		value: () => mark_redirected(response_clone.call(response))
	});
	return response;
};

/**
 * Whether a `body` given in `init` is one a redirect can't replay, a `ReadableStream`
 * or other async iterable, the shapes `Request` treats as having no source to read again.
 */
const is_stream_body = (body: BodyInit | null | undefined): boolean =>
	typeof body === 'object' &&
	body !== null &&
	typeof (body as unknown as AsyncIterable<unknown>)[Symbol.asyncIterator] === 'function';

/**
 * Sends `request` with its redirect mode and dispatcher set by this module. undici's init
 * dictionary reads a key the init doesn't own through `Object.prototype`, even for a null-prototype
 * init, so this names those two and the request's own fields a polluted key would replace.
 * `body: null` keeps the `Request`'s body. A non-empty init resets a `Request`'s referrer.
 */
const send_request = (request: Request, redirect: RequestRedirect): Promise<Response> =>
	native_fetch(
		request,
		create_record({
			redirect,
			dispatcher: HOST_DISPATCHER,
			referrer: request.referrer,
			referrerPolicy: request.referrerPolicy,
			signal: request.signal,
			method: request.method,
			headers: request.headers,
			body: null,
			cache: request.cache,
			integrity: request.integrity
		})
	);

/**
 * The body bytes for a redirect's replay, or rejecting with the signal's reason,
 * since a stream a server answered before reading to its end may never finish.
 */
const read_replay = (replay: Request, signal: AbortSignal): Promise<ArrayBuffer> =>
	new Promise((resolve, reject) => {
		const on_abort = (): void => {
			// eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors -- a signal's reason is the caller's, any value
			reject(signal.reason);
		};
		if (signal.aborted) {
			on_abort();
			return;
		}
		signal.addEventListener('abort', on_abort, { once: true });
		void replay
			.arrayBuffer()
			.then(resolve, reject)
			.finally(() => signal.removeEventListener('abort', on_abort));
	});

const follow_redirects = async (
	allowed: Allowed | null,
	first: Request,
	first_url: URL,
	body_is_stream: boolean
): Promise<Response> => {
	let has_body = first.body !== null;
	let stream_body = has_body && body_is_stream;
	// the replay branch, read only if a hop redirects and released once a response settles
	let replay = has_body && !stream_body ? first.clone() : null;
	try {
		let send = first;
		let url = first_url;
		let method = first.method;
		const headers = new HeadersClass(first.headers);
		let body: ArrayBuffer | null = null;
		for (let hops = 0; ; hops++) {
			const response = await send_request(send, 'manual');
			const next = to_redirect_url(response, url);
			if (next === null) return hops === 0 ? response : mark_redirected(response);
			await response.body?.cancel().catch(ignore_error);
			check_url(allowed, next);
			if (hops === MAX_REDIRECTS) {
				throw new TypeError(`too many redirects, more than ${MAX_REDIRECTS}`);
			}
			if (first.mode === 'same-origin' && next.origin !== first_url.origin) {
				throw new TypeError("a `same-origin` request can't redirect to another origin");
			}
			const { status } = response;
			if (stream_body && status !== 303) {
				throw new TypeError(`a stream body can't replay on a ${status} redirect`);
			}
			if (
				(status === 303 && method !== 'GET' && method !== 'HEAD') ||
				((status === 301 || status === 302) && method === 'POST')
			) {
				method = 'GET';
				has_body = false;
				stream_body = false;
				body = null;
				for (const name of BODY_HEADERS) headers.delete(name);
			}
			if (next.origin !== url.origin) {
				for (const name of CROSS_ORIGIN_HEADERS) headers.delete(name);
			}
			// bytes, not the stream, so the hop keeps `Content-Length` and `keepalive`
			if (has_body && replay !== null) {
				body = await read_replay(replay, first.signal);
				replay = null;
			}
			url = next;
			send = new RequestClass(
				next.href,
				create_record<RequestInit>({
					method,
					headers,
					body,
					signal: first.signal,
					credentials: first.credentials,
					mode: first.mode,
					cache: first.cache,
					referrer: first.referrer,
					referrerPolicy: first.referrerPolicy,
					integrity: first.integrity,
					keepalive: first.keepalive,
					redirect: 'manual'
				})
			);
		}
	} finally {
		void replay?.body?.cancel().catch(ignore_error);
	}
};

/**
 * Sends one request built from the input once, so a stateful `toString` can't differ between
 * the check and the send.
 */
const net_fetch = async (
	allowed: Allowed | null,
	input: string | URL | Request,
	init?: RequestInit
): Promise<Response> => {
	const request = new RequestClass(input, init);
	const url = new URLClass(request.url);
	check_url(allowed, url);
	if (request.redirect !== 'follow') return send_request(request, request.redirect);
	return follow_redirects(allowed, request, url, is_stream_body(init?.body));
};

class NodeNet implements Net {
	readonly fetch: Net['fetch'];
	readonly #allowed: Allowed | null;

	constructor(key: symbol, allowed: Allowed | null) {
		check_handle_key(key);
		this.#allowed = allowed;
		this.fetch = freeze((input: string | URL | Request, init?: RequestInit) =>
			net_fetch(allowed, input, init)
		);
	}

	narrow(options: NetNarrowOptions): Net {
		const parent = this.#allowed;
		const hosts = read_own(options, 'hosts');
		if (!isArray(hosts)) throw new TypeError('net.narrow takes `{hosts: [...]}`');
		const allowed = create_record<Record<string, true>>({});
		const length = hosts.length;
		for (let i = 0; i < length; i++) {
			const { hostname, key } = parse_host(hosts[i]);
			if (parent !== null && parent[key] !== true && parent[hostname] !== true) {
				throw new NetError(
					'not_granted',
					key,
					`not_granted: ${key} isn't in this net's hosts, and narrow only shrinks`
				);
			}
			allowed[key] = true;
		}
		return freeze(new NodeNet(HANDLE_KEY, freeze(allowed)));
	}
}

// frozen, null-prototype, so a polluted `Object.prototype.then` can't make `await` adopt a forged value
setPrototypeOf(NodeNet.prototype, null);
freeze(NodeNet.prototype);
freeze(NodeNet);

/**
 * The net host module's root handle. Import it where authority enters (a task, a script,
 * an entry module) and hand a narrower `net.narrow({hosts})` down.
 */
export const net: Net = freeze(new NodeNet(HANDLE_KEY, null));
