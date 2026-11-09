import { describe, test, assert, beforeEach, afterEach, vi } from 'vitest';
import * as http from 'node:http';

import { fetch_value } from '@fuzdev/fuz_util/fetch.ts';

import { net, type Net } from '#lib/net.ts';
import { is_net_error } from '#lib/net_error.ts';

vi.setConfig({ testTimeout: 30_000 });

/*

Loopback servers only, on ephemeral ports. `a` is the origin a request starts at,
`b` the cross-origin target of its redirects, and every hit each one sees is recorded, so a
test can assert what reached a host. `localhost` and `127.0.0.1` are distinct hostnames for the same
loopback, so a host grant can tell them apart.

*/

interface Hit {
	server: 'a' | 'b';
	method: string;
	url: string;
	body: string;
	headers: http.IncomingHttpHeaders;
}

interface TestServer {
	name: 'a' | 'b';
	port: number;
	origin: string;
	close: () => void;
}

let hits: Array<Hit>;
let closed: Array<string>;
let a: TestServer;
let b: TestServer;
let A: string;
let B: string;

const start_server = (name: 'a' | 'b'): Promise<TestServer> =>
	new Promise((resolve) => {
		const server = http.createServer((req, res) => {
			// answers before the body ends, so what's sent can outlive the response
			if (req.url!.startsWith('/early')) {
				hits.push({
					server: name,
					method: req.method!,
					url: req.url!,
					body: '',
					headers: req.headers
				});
				route(req, res);
				req.resume();
				return;
			}
			let body = '';
			req.setEncoding('utf8');
			req.on('data', (chunk) => (body += chunk));
			req.on('end', () => {
				hits.push({ server: name, method: req.method!, url: req.url!, body, headers: req.headers });
				route(req, res);
			});
		});
		server.listen(0, '127.0.0.1', () => {
			const { port } = server.address() as { port: number };
			resolve({
				name,
				port,
				origin: `http://127.0.0.1:${port}`,
				close: () => {
					server.closeAllConnections();
					server.close();
				}
			});
		});
	});

const route = (req: http.IncomingMessage, res: http.ServerResponse): void => {
	const url = new URL(req.url!, 'http://x');
	if (url.pathname === '/stall') return; // never answers, for the abort test
	if (url.pathname === '/redirect_stall') {
		// a redirect whose body never ends, so the hop has to cancel it
		res.statusCode = 302;
		res.setHeader('location', url.searchParams.get('l')!);
		res.on('close', () => closed.push(url.pathname));
		res.write('still sending');
		return;
	}
	if (url.pathname === '/early') {
		// answers before the request's body ends, for the replay tests
		res.statusCode = Number(url.searchParams.get('s') ?? '200');
		const location = url.searchParams.get('l');
		if (location !== null) res.setHeader('location', location);
		res.end();
		return;
	}
	if (url.pathname === '/loop') {
		res.statusCode = 302;
		res.setHeader('location', `/loop?n=${Number(url.searchParams.get('n')) + 1}`);
		res.end();
		return;
	}
	if (url.pathname.startsWith('/r')) {
		res.statusCode = Number(url.pathname.slice(2, 5));
		const location = url.searchParams.get('l');
		if (location !== null) res.setHeader('location', location);
		const raw = url.searchParams.get('raw');
		if (raw !== null) res.setHeader('location', Buffer.from(raw, 'utf8').toString('latin1'));
		res.end('redirect body');
		return;
	}
	res.setHeader('content-type', 'text/plain');
	res.end(`final ${url.pathname}`);
};

beforeEach(async () => {
	hits = [];
	closed = [];
	a = await start_server('a');
	b = await start_server('b');
	A = a.origin;
	B = `http://localhost:${b.port}`;
});

afterEach(() => {
	a.close();
	b.close();
});

const redirect = (status: number, location: string): string =>
	`${A}/r${status}?l=${encodeURIComponent(location)}`;

const granted = (): Net => net.narrow({ hosts: ['127.0.0.1', 'localhost'] });

const hits_on = (server: 'a' | 'b'): Array<Hit> => hits.filter((h) => h.server === server);

const assert_rejects_not_granted = async (promise: Promise<unknown>): Promise<void> => {
	try {
		await promise;
	} catch (error) {
		assert.ok(is_net_error(error, 'not_granted'), `expected not_granted, got ${String(error)}`);
		return;
	}
	assert.fail('expected not_granted');
};

// reroutes every request to the `b` server through undici's own global dispatcher
const rerouting_dispatcher = () => ({
	dispatch: (options: Record<string, unknown>, handler: unknown): boolean =>
		(globalThis as any)[Symbol.for('undici.globalDispatcher.1')].dispatch(
			{ ...options, origin: b.origin },
			handler
		)
});

const stream_body = (): ReadableStream =>
	new ReadableStream({
		start(controller) {
			controller.enqueue(new TextEncoder().encode('streamed'));
			controller.close();
		}
	});

const to_rejection = async (promise: Promise<unknown>): Promise<unknown> => {
	try {
		await promise;
	} catch (error) {
		return error;
	}
	return assert.fail('expected a rejection');
};

const assert_rejects_type = async (promise: Promise<unknown>): Promise<void> => {
	const error = await to_rejection(promise);
	assert.instanceOf(error, TypeError);
	assert.notOk(is_net_error(error));
};

describe('redirects refused', () => {
	test('a `Location` at a refused host sends nothing to it', async () => {
		await assert_rejects_not_granted(
			net.narrow({ hosts: [`127.0.0.1:${a.port}`] }).fetch(redirect(302, `${B}/x`))
		);
		assert.strictEqual(hits_on('a').length, 1);
		assert.strictEqual(hits_on('b').length, 0);
	});

	test('a later hop to a refused host sends nothing to it', async () => {
		const first = redirect(302, redirect(302, `${B}/x`));
		await assert_rejects_not_granted(net.narrow({ hosts: [`127.0.0.1:${a.port}`] }).fetch(first));
		assert.strictEqual(hits_on('a').length, 2);
		assert.strictEqual(hits_on('b').length, 0);
	});

	test('a port grant refuses another port on the same hostname', async () => {
		const to_b_port = `http://127.0.0.1:${b.port}/x`;
		await assert_rejects_not_granted(
			net.narrow({ hosts: [`127.0.0.1:${a.port}`] }).fetch(redirect(302, to_b_port))
		);
		assert.strictEqual(hits_on('b').length, 0);
	});

	test('a `Location` with another scheme is `not_granted`', async () => {
		for (const location of ['data:,x', 'file:///etc/passwd', 'ftp://example.com/']) {
			await assert_rejects_not_granted(granted().fetch(redirect(302, location)));
		}
	});

	test('`manual` and `error` pass through, `manual` showing the `Location`', async () => {
		const handle = net.narrow({ hosts: [`127.0.0.1:${a.port}`] });
		const res = await handle.fetch(redirect(302, `${B}/x`), { redirect: 'manual' });
		assert.strictEqual(res.status, 302);
		assert.strictEqual(res.headers.get('location'), `${B}/x`);
		await assert_rejects_type(handle.fetch(redirect(302, `${B}/x`), { redirect: 'error' }));
		assert.strictEqual(hits_on('b').length, 0);
	});

	test('a `Request` with `manual` passes through', async () => {
		const handle = net.narrow({ hosts: [`127.0.0.1:${a.port}`] });
		const res = await handle.fetch(new Request(redirect(302, `${B}/x`), { redirect: 'manual' }));
		assert.strictEqual(res.status, 302);
	});

	test('the first request at a refused host sends nothing', async () => {
		await assert_rejects_not_granted(net.narrow({ hosts: ['example.com'] }).fetch(`${A}/x`));
		assert.strictEqual(hits.length, 0);
	});
});

describe('headers and methods across a hop', () => {
	const SECRETS = { authorization: 'Bearer t', cookie: 'a=1', 'proxy-authorization': 'Basic p' };

	test('credentials are stripped on a cross-origin hop', async () => {
		const res = await granted().fetch(redirect(302, `${B}/x`), {
			headers: { ...SECRETS, 'x-keep': 'kept' }
		});
		assert.strictEqual(res.status, 200);
		const [first] = hits_on('a');
		assert.strictEqual(first!.headers.authorization, 'Bearer t');
		const [second] = hits_on('b');
		assert.isUndefined(second!.headers.authorization);
		assert.isUndefined(second!.headers.cookie);
		assert.isUndefined(second!.headers['proxy-authorization']);
		assert.strictEqual(second!.headers.host, `localhost:${b.port}`);
		assert.strictEqual(second!.headers['x-keep'], 'kept');
	});

	test('a caller `Host` never reaches the wire', async () => {
		await granted().fetch(`${A}/x`, { headers: { host: 'steered.example' } });
		assert.strictEqual(hits[0]!.headers.host, `127.0.0.1:${a.port}`);
	});

	test('a different port or scheme is a different origin', async () => {
		await granted().fetch(redirect(302, `http://127.0.0.1:${b.port}/x`), { headers: SECRETS });
		assert.isUndefined(hits_on('b')[0]!.headers.authorization);
	});

	test('credentials stay on a same-origin hop', async () => {
		await granted().fetch(redirect(302, '/final'), { headers: SECRETS });
		assert.strictEqual(hits_on('a')[1]!.headers.authorization, 'Bearer t');
		assert.strictEqual(hits_on('a')[1]!.headers.cookie, 'a=1');
	});

	test('credentials stay gone after returning to the first origin', async () => {
		await granted().fetch(redirect(302, `${B}/r302?l=${encodeURIComponent(`${A}/final`)}`), {
			headers: SECRETS
		});
		assert.strictEqual(hits_on('a').length, 2);
		assert.isUndefined(hits_on('a')[1]!.headers.authorization);
	});

	test('a 303 becomes GET and drops the body and its headers', async () => {
		await granted().fetch(redirect(303, '/final'), {
			method: 'POST',
			body: 'secret body',
			headers: { 'content-type': 'text/x', 'x-keep': 'kept' }
		});
		const [, second] = hits_on('a');
		assert.strictEqual(second!.method, 'GET');
		assert.strictEqual(second!.body, '');
		assert.isUndefined(second!.headers['content-type']);
		assert.strictEqual(second!.headers['x-keep'], 'kept');
	});

	test('a 307 replays the body and keeps its length', async () => {
		const res = await granted().fetch(redirect(307, '/final'), {
			method: 'POST',
			body: 'replayed body'
		});
		assert.strictEqual(res.status, 200);
		const [first, second] = hits_on('a');
		assert.strictEqual(first!.body, 'replayed body');
		assert.strictEqual(second!.method, 'POST');
		assert.strictEqual(second!.body, 'replayed body');
		assert.strictEqual(second!.headers['content-length'], '13');
		assert.isUndefined(second!.headers['transfer-encoding']);
	});

	test('a `Request` body replays too, across hops', async () => {
		const request = new Request(redirect(307, redirect(308, '/final')), {
			method: 'PUT',
			body: 'replayed body'
		});
		await granted().fetch(request);
		const [, second, third] = hits_on('a');
		assert.strictEqual(second!.body, 'replayed body');
		assert.strictEqual(third!.body, 'replayed body');
	});

	test('a `Request` input keeps its method, headers, body, and signal', async () => {
		const res = await granted().fetch(
			new Request(`${A}/final`, { method: 'PUT', body: 'x', headers: { 'x-keep': 'kept' } })
		);
		assert.strictEqual(res.status, 200);
		assert.strictEqual(hits[0]!.method, 'PUT');
		assert.strictEqual(hits[0]!.body, 'x');
		assert.strictEqual(hits[0]!.headers['x-keep'], 'kept');
	});

	test('a stream body is refused on a 307, sending nothing further', async () => {
		await assert_rejects_type(
			granted().fetch(redirect(307, `${B}/x`), {
				method: 'POST',
				body: stream_body(),
				duplex: 'half'
			} as RequestInit)
		);
		assert.strictEqual(hits_on('b').length, 0);
	});

	test('a stream POST a 303 turns to GET follows a later 307', async () => {
		const location = `/r307?l=${encodeURIComponent('/final')}`;
		const res = await granted().fetch(redirect(303, location), {
			method: 'POST',
			body: stream_body(),
			duplex: 'half'
		} as RequestInit);
		assert.strictEqual(res.status, 200);
		assert.strictEqual(hits_on('a').at(-1)!.method, 'GET');
	});

	test('a redirect whose body never ends is cancelled', async () => {
		const res = await granted().fetch(`${A}/redirect_stall?l=${encodeURIComponent('/final')}`);
		assert.strictEqual(res.status, 200);
		await vi.waitFor(() => assert.deepEqual(closed, ['/redirect_stall']));
	});

	test('a stream body on a 303 goes GET without it', async () => {
		const res = await granted().fetch(redirect(303, `${B}/x`), {
			method: 'POST',
			body: stream_body(),
			duplex: 'half'
		} as RequestInit);
		assert.strictEqual(res.status, 200);
		assert.strictEqual(hits_on('b')[0]!.method, 'GET');
	});

	test('more than 20 redirects is refused, 21 requests sent', async () => {
		await assert_rejects_type(granted().fetch(`${A}/loop?n=0`));
		assert.strictEqual(hits_on('a').length, 21);
	});

	test('`signal` reaches a later hop', async () => {
		const controller = new AbortController();
		const reason = new Error('stopped');
		const promise = granted().fetch(redirect(302, '/stall'), { signal: controller.signal });
		while (hits_on('a').length < 2) await new Promise((resolve) => setTimeout(resolve, 5));
		controller.abort(reason);
		const caught = await to_rejection(promise);
		assert.strictEqual(caught, reason);
	});

	test('`redirected` and `url` hold on the final response and its clone', async () => {
		const res = await granted().fetch(redirect(302, `${B}/x`));
		assert.ok(res.redirected);
		assert.strictEqual(res.url, `${B}/x`);
		assert.instanceOf(res, Response);
		const clone = res.clone();
		assert.ok(clone.redirected);
		assert.strictEqual(clone.url, res.url);
		assert.strictEqual(await res.text(), 'final /x');
		assert.strictEqual(await clone.text(), 'final /x');
	});

	test('a response with no hop keeps its own `redirected`', async () => {
		const res = await granted().fetch(`${A}/final`);
		assert.notOk(res.redirected);
		assert.notOk(Object.hasOwn(res, 'redirected'));
	});
});

describe('`Object.prototype` on the send', () => {
	const polluted = async (key: string, value: unknown, fn: () => Promise<unknown>) => {
		(Object.prototype as any)[key] = value;
		try {
			await fn();
		} finally {
			delete (Object.prototype as any)[key];
		}
	};

	const narrow_a = (): Net => net.narrow({ hosts: [`127.0.0.1:${a.port}`] });

	test('`redirect` cannot switch a followed request to native follow', async () => {
		await polluted('redirect', 'follow', () =>
			assert_rejects_not_granted(narrow_a().fetch(redirect(302, `${B}/x`)))
		);
		assert.strictEqual(hits_on('b').length, 0);
	});

	test('`redirect` cannot switch a `manual` request to native follow', async () => {
		await polluted('redirect', 'follow', async () => {
			const res = await narrow_a().fetch(redirect(302, `${B}/x`), { redirect: 'manual' });
			assert.strictEqual(res.status, 302);
		});
		assert.strictEqual(hits_on('b').length, 0);
	});

	test('`dispatcher` cannot reroute a send', async () => {
		await polluted('dispatcher', rerouting_dispatcher(), async () => {
			// undici 8 rejects a polluted `dispatcher` on its own dispatch, undici 7 ignores it
			await narrow_a()
				.fetch(`${A}/x`)
				.catch(() => null);
		});
		assert.strictEqual(hits_on('b').length, 0);
	});

	test('`method` and `body` cannot change a send or a hop', async () => {
		await polluted('method', 'DELETE', () =>
			polluted('body', 'polluted', () =>
				narrow_a().fetch(redirect(307, '/final'), { method: 'POST', body: 'real' })
			)
		);
		assert.deepEqual(
			hits.map((h) => [h.method, h.body]),
			[
				['POST', 'real'],
				['POST', 'real']
			]
		);
	});
});

describe('a polluted `signal`', () => {
	const narrow_a = (): Net => net.narrow({ hosts: [`127.0.0.1:${a.port}`] });

	const assert_caller_aborts = async (input: string, hits_before_abort: number): Promise<void> => {
		const controller = new AbortController();
		const reason = new Error('stopped');
		(Object.prototype as any).signal = new AbortController().signal;
		try {
			const promise = narrow_a().fetch(input, { signal: controller.signal });
			while (hits_on('a').length < hits_before_abort) {
				await new Promise((resolve) => setTimeout(resolve, 5));
			}
			controller.abort(reason);
			assert.strictEqual(await to_rejection(promise), reason);
		} finally {
			delete (Object.prototype as any).signal;
		}
	};

	test("doesn't replace the caller's on a first send", async () => {
		await assert_caller_aborts(`${A}/stall`, 1);
	});

	test("doesn't replace the caller's on a hop", async () => {
		await assert_caller_aborts(redirect(302, '/stall'), 2);
	});
});

describe('`Object.prototype` on the keys a send names', () => {
	const narrow_a = (): Net => net.narrow({ hosts: [`127.0.0.1:${a.port}`] });

	// each key with a value a send wouldn't survive, and an init owning that key
	const KEYS: Array<[string, unknown, RequestInit]> = [
		['integrity', 'sha256-AAAA', { integrity: '' }],
		['headers', { 'x-polluted': 'yes' }, { headers: { 'x-own': 'kept' } }],
		['cache', 'no-store', { cache: 'default' }]
	];

	for (const [key, value, init] of KEYS) {
		test(`a polluted \`${key}\` doesn't reach the send`, async () => {
			(Object.prototype as any)[key] = value;
			let status: number;
			try {
				status = (await narrow_a().fetch(`${A}/x`, init)).status;
			} finally {
				delete (Object.prototype as any)[key];
			}
			assert.strictEqual(status, 200);
			assert.isUndefined(hits[0]!.headers['x-polluted']);
			assert.notStrictEqual(hits[0]!.headers['cache-control'], 'no-cache');
		});
	}
});

describe('`dispatcher`', () => {
	const narrow_a = (): Net => net.narrow({ hosts: [`127.0.0.1:${a.port}`] });

	for (const [name, make] of [
		[
			'in `init`',
			() => narrow_a().fetch(`${A}/x`, { dispatcher: rerouting_dispatcher() } as RequestInit)
		],
		[
			'on a `Request`',
			() =>
				narrow_a().fetch(
					new Request(`${A}/x`, { dispatcher: rerouting_dispatcher() } as RequestInit)
				)
		],
		[
			'with `manual`',
			() =>
				narrow_a().fetch(`${A}/x`, {
					dispatcher: rerouting_dispatcher(),
					redirect: 'manual'
				} as RequestInit)
		]
	] as const) {
		test(`${name} cannot reroute it`, async () => {
			const res = await make();
			assert.strictEqual(res.status, 200);
			assert.deepEqual(
				hits.map((h) => h.server),
				['a']
			);
		});
	}

	const global_dispatcher_key = (): symbol =>
		Symbol.for(
			`undici.globalDispatcher.${Number(globalThis.process.versions.undici!.split('.')[0]) >= 8 ? 2 : 1}`
		);

	test('a dispatcher set globally still runs, as a proxy would', async () => {
		const key = global_dispatcher_key();
		const real = (globalThis as any)[key];
		let dispatched = 0;
		(globalThis as any)[key] = {
			dispatch: (options: unknown, handler: unknown) => {
				dispatched++;
				return real.dispatch(options, handler);
			}
		};
		try {
			const res = await narrow_a().fetch(`${A}/x`);
			assert.strictEqual(res.status, 200);
			assert.strictEqual(dispatched, 1);
		} finally {
			(globalThis as any)[key] = real;
		}
	});

	test('a missing global dispatcher rejects, sending nothing', async () => {
		const key = global_dispatcher_key();
		const real = (globalThis as any)[key];
		(globalThis as any)[key] = undefined;
		try {
			const error = await to_rejection(narrow_a().fetch(`${A}/x`));
			assert.instanceOf(error, TypeError);
			assert.include(String(error.cause), "undici's global dispatcher is missing");
			assert.strictEqual(hits.length, 0);
		} finally {
			(globalThis as any)[key] = real;
		}
	});
});

describe('a redirect failure', () => {
	test('an invalid `Location` holds no URL', async () => {
		const location = 'http://[bad/?token=SECRET';
		const error = await to_rejection(granted().fetch(redirect(302, location)));
		assert.instanceOf(error, TypeError);
		assert.isUndefined(error.cause);
		assert.notInclude(String(error), 'SECRET');
	});
});

describe('the replay', () => {
	test('a `Request` stream a server redirects early rejects on abort', async () => {
		const controller = new AbortController();
		setTimeout(() => controller.abort(new Error('stopped')), 50);
		const request = new Request(`${A}/early?s=307&l=/next`, {
			method: 'POST',
			// a chunk so the request goes out, then no end, so only the abort stops the read
			body: new ReadableStream({
				start: (stream) => {
					stream.enqueue(new Uint8Array(16));
				}
			}),
			duplex: 'half',
			signal: controller.signal
		} as RequestInit);
		const error = await to_rejection(granted().fetch(request));
		assert.instanceOf(error, Error);
		assert.strictEqual(error.message, 'stopped');
	});

	test('the replay branch is released when no redirect comes', async () => {
		const clone = vi.spyOn(Request.prototype, 'clone');
		try {
			const res = await granted().fetch(`${A}/final`, { method: 'POST', body: 'hello' });
			assert.strictEqual(res.status, 200);
			assert.strictEqual(clone.mock.results.length, 1);
			const replay = clone.mock.results[0]!.value as Request;
			await vi.waitFor(() => assert.ok(replay.bodyUsed));
		} finally {
			clone.mockRestore();
		}
	});
});

describe('a replaced global `fetch`', () => {
	test('is not called after this module loads', async () => {
		const stub = vi.fn();
		vi.stubGlobal('fetch', stub);
		try {
			const res = await net.fetch(`${A}/x`);
			assert.strictEqual(res.status, 200);
			assert.strictEqual(stub.mock.calls.length, 0);
			assert.strictEqual(hits.length, 1);
		} finally {
			vi.unstubAllGlobals();
		}
	});
});

describe('building the request once', () => {
	test('a stateful URL object is read once, and what is sent is what was checked', async () => {
		let calls = 0;
		const stateful = {
			toString: () => (calls++ === 0 ? `${A}/first` : 'http://example.com/evil')
		};
		const res = await net.narrow({ hosts: ['127.0.0.1'] }).fetch(stateful as any);
		assert.strictEqual(res.status, 200);
		assert.strictEqual(calls, 1);
		assert.strictEqual(hits.length, 1);
		assert.strictEqual(hits[0]!.url, '/first');
	});

	test('a stateful URL subclass cannot differ between the check and the send', async () => {
		let calls = 0;
		class Stateful extends URL {
			override toString(): string {
				return calls++ === 0 ? `${A}/first` : 'http://example.com/evil';
			}
		}
		await net.narrow({ hosts: ['127.0.0.1'] }).fetch(new Stateful('http://example.com/'));
		assert.strictEqual(calls, 1);
		assert.strictEqual(hits[0]!.url, '/first');
	});
});

describe('documented differences from native follow', () => {
	test('a `Request` carrying a stream body replays, its source not being observable', async () => {
		const request = new Request(redirect(307, '/final'), {
			method: 'POST',
			body: stream_body(),
			duplex: 'half'
		} as RequestInit);
		const res = await granted().fetch(request);
		assert.strictEqual(res.status, 200);
		assert.strictEqual(hits_on('a')[1]!.body, 'streamed');
	});

	test('`integrity` stays on a hop, so a matching redirect body is no way past it', async () => {
		const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode('redirect body'));
		const integrity = `sha256-${Buffer.from(digest).toString('base64')}`;
		await assert_rejects_type(granted().fetch(redirect(302, '/final'), { integrity }));
		assert.strictEqual(hits_on('a').length, 2);
	});

	test('`integrity` on a redirected request rejects', async () => {
		await assert_rejects_type(
			granted().fetch(redirect(302, '/final'), { integrity: 'sha256-AAAA' })
		);
	});

	test('a `no-cors` response after a cross-origin hop is readable', async () => {
		const res = await granted().fetch(redirect(302, `${B}/x`), { mode: 'no-cors' });
		assert.strictEqual(res.status, 200);
		assert.strictEqual(res.type, 'basic');
		assert.strictEqual(await res.text(), 'final /x');
	});

	test('a response after a cross-origin hop is `basic`', async () => {
		const res = await granted().fetch(redirect(302, `${B}/x`));
		assert.strictEqual(res.type, 'basic');
	});
});

describe('`fetch_value`', () => {
	test('composes with `net.fetch` directly', async () => {
		const result = await fetch_value<string>(redirect(302, `${B}/x`), {
			fetch: granted().fetch,
			token: 'tok',
			parse: (value) => value
		});
		assert.ok(result.ok);
		assert.strictEqual(hits_on('a')[0]!.headers.authorization, 'Bearer tok');
		assert.isUndefined(hits_on('b')[0]!.headers.authorization);
		assert.strictEqual(hits_on('b')[0]!.headers.accept, 'application/json');
	});

	test('a redirect to a refused host refuses', async () => {
		await assert_rejects_not_granted(
			fetch_value(redirect(302, `${B}/x`), {
				fetch: net.narrow({ hosts: [`127.0.0.1:${a.port}`] }).fetch
			})
		);
		assert.strictEqual(hits_on('b').length, 0);
	});

	test('a POST body through a 307 replays', async () => {
		const result = await fetch_value(redirect(307, '/final'), {
			fetch: granted().fetch,
			params: { a: 1 } as any,
			parse: (value) => value
		});
		assert.ok(result.ok);
		assert.strictEqual(hits_on('a')[1]!.body, '{"a":1}');
	});
});

/**
 * What a request did: what the outcome was, and what each server saw.
 */
interface Observed {
	outcome: unknown;
	hits: Array<unknown>;
}

const observe = async (
	fetch_fn: typeof globalThis.fetch,
	input: () => string | Request,
	init: () => RequestInit | undefined
): Promise<Observed> => {
	hits.length = 0;
	let outcome: unknown;
	try {
		const res = await fetch_fn(input(), init());
		outcome = { status: res.status, redirected: res.redirected, url: res.url };
		await res.body?.cancel();
	} catch (error) {
		assert.notOk(is_net_error(error), `native follow can't be not_granted: ${String(error)}`);
		outcome = (error as Error).name;
	}
	return {
		outcome,
		hits: hits.map((hit) => ({
			server: hit.server,
			method: hit.method,
			url: hit.url,
			body: hit.body,
			authorization: hit.headers.authorization,
			cookie: hit.headers.cookie,
			proxy_authorization: hit.headers['proxy-authorization'],
			keep: hit.headers['x-keep'],
			content_type: hit.headers['content-type'],
			content_length: hit.headers['content-length'],
			transfer_encoding: hit.headers['transfer-encoding'],
			referer: hit.headers.referer,
			origin: hit.headers.origin,
			sec_fetch_mode: hit.headers['sec-fetch-mode'],
			cache_control: hit.headers['cache-control'],
			pragma: hit.headers.pragma
		}))
	};
};

describe('matches native follow', () => {
	const CROSS = {
		authorization: 'Bearer t',
		cookie: 'a=1',
		'proxy-authorization': 'p',
		'x-keep': 'k'
	};
	const body_init = (method: string): RequestInit => ({
		method,
		body: 'hello',
		headers: { 'content-type': 'text/x', 'content-language': 'en', 'x-keep': 'k' }
	});

	interface Case {
		name: string;
		input: () => string | Request;
		init?: () => RequestInit | undefined;
	}

	const cases: Array<Case> = [];
	for (const status of [301, 302, 303, 307, 308]) {
		for (const method of ['POST', 'PUT', 'PATCH', 'DELETE']) {
			cases.push({
				name: `${status} ${method}`,
				input: () => redirect(status, '/final'),
				init: () => body_init(method)
			});
		}
		for (const method of ['GET', 'HEAD']) {
			cases.push({
				name: `${status} ${method}`,
				input: () => redirect(status, '/final'),
				init: () => ({ method, headers: { 'x-keep': 'k' } })
			});
		}
		cases.push({
			name: `${status} cross-origin credentials`,
			input: () => redirect(status, `${B}/x`),
			init: () => ({ headers: CROSS })
		});
		cases.push({
			name: `${status} same-origin credentials`,
			input: () => redirect(status, `${A}/x`),
			init: () => ({ headers: CROSS })
		});
		cases.push({
			name: `${status} stream POST`,
			input: () => redirect(status, '/final'),
			init: () => ({ method: 'POST', body: stream_body(), duplex: 'half' }) as RequestInit
		});
		cases.push({
			name: `${status} Request string POST`,
			input: () => new Request(redirect(status, '/final'), { method: 'POST', body: 'hello' })
		});
	}
	cases.push(
		{ name: 'relative Location', input: () => redirect(302, 'final') },
		{ name: 'absolute-path Location', input: () => redirect(302, '/final') },
		{
			name: 'a relative Location resolves against its own response on a later hop',
			input: () => redirect(302, `${B}/r302?l=final`)
		},
		{ name: 'fragment Location', input: () => redirect(302, '/final%23frag') },
		{
			name: 'no Location, returned as-is',
			input: () => `${A}/r302`
		},
		{ name: 'invalid Location', input: () => redirect(302, 'http://[bad') },
		{ name: 'empty Location loops', input: () => redirect(302, '') },
		{
			name: 'credentials in Location',
			input: () => redirect(302, `http://u:p@localhost:${b.port}/x`)
		},
		{
			name: 'non-ASCII Location',
			input: () => `${A}/r302?raw=${encodeURIComponent('/é')}`
		},
		{ name: '300 returned', input: () => redirect(300, '/final') },
		{ name: '304 returned', input: () => redirect(304, '/final') },
		{ name: 'hop cap', input: () => `${A}/loop?n=0` },
		{
			name: 'same-origin mode, same origin',
			input: () => redirect(302, '/final'),
			init: () => ({ mode: 'same-origin' })
		},
		{
			name: 'same-origin mode, cross origin',
			input: () => redirect(302, `${B}/x`),
			init: () => ({ mode: 'same-origin' })
		},
		{
			name: 'referrer stripped by default policy across origins',
			input: () => redirect(302, `${B}/x`),
			init: () => ({ referrer: `${A}/page` })
		},
		{
			name: 'referrer kept on a same-origin hop',
			input: () => redirect(302, '/final'),
			init: () => ({ referrer: `${A}/page` })
		},
		{
			name: 'redirect error',
			input: () => redirect(302, '/final'),
			init: () => ({ redirect: 'error' })
		},
		{
			name: 'redirect manual',
			input: () => redirect(302, '/final'),
			init: () => ({ redirect: 'manual' })
		},
		{
			name: 'cache and credentials survive',
			input: () => redirect(302, '/final'),
			init: () => ({ cache: 'no-store', credentials: 'omit' })
		},
		{
			name: 'FormData body on a 307',
			input: () => redirect(307, '/final'),
			init: () => {
				const body = new FormData();
				body.set('a', 'b');
				return { method: 'POST', body };
			}
		},
		{
			name: 'URLSearchParams body on a 307',
			input: () => redirect(307, '/final'),
			init: () => ({ method: 'POST', body: new URLSearchParams({ a: 'b' }) })
		},
		{
			name: 'Blob body on a 308',
			input: () => redirect(308, '/final'),
			init: () => ({ method: 'POST', body: new Blob(['blob'], { type: 'text/x' }) })
		},
		{
			name: 'bytes body on a 307',
			input: () => redirect(307, '/final'),
			init: () => ({ method: 'POST', body: new Uint8Array([104, 105]) })
		}
	);

	const normalize = (observed: Observed): Observed =>
		JSON.parse(
			JSON.stringify(observed).replaceAll(/boundary=-*[\w-]+|-*formdata-undici-\d+/g, 'X')
		);

	for (const { name, input, init = () => undefined } of cases) {
		test(name, async () => {
			const native_observed = await observe(fetch, input, init);
			const net_observed = await observe(granted().fetch, input, init);
			assert.deepEqual(normalize(net_observed), normalize(native_observed));
		});
	}
});
