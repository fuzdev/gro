import { describe, test, assert, vi, afterEach } from 'vitest';

import { net, type Net } from '#lib/net.ts';
import { NetError, is_net_error, type NetErrorKind } from '#lib/net_error.ts';

const assert_net_error = (fn: () => unknown, kind: NetErrorKind): NetError => {
	try {
		fn();
	} catch (error) {
		assert.ok(is_net_error(error, kind), `expected ${kind}, got ${String(error)}`);
		return error;
	}
	assert.fail('expected a NetError');
};

const assert_throws_type = (fn: () => unknown): void => {
	assert.throws(fn, TypeError);
};

const assert_rejects_net_error = async (
	promise: Promise<unknown>,
	kind: NetErrorKind
): Promise<NetError> => {
	try {
		await promise;
	} catch (error) {
		assert.ok(is_net_error(error, kind), `expected ${kind}, got ${String(error)}`);
		return error;
	}
	assert.fail('expected a NetError');
};

// a handle reaching only `hosts`, chained from the root so every narrow is exercised
const narrowed = (hosts: Array<string>): Net => net.narrow({ hosts });

describe('handles', () => {
	test('`net` and a narrowed handle are frozen, their `fetch` an own frozen closure', () => {
		for (const handle of [net, narrowed(['127.0.0.1'])]) {
			assert.ok(Object.isFrozen(handle));
			assert.ok(Object.hasOwn(handle, 'fetch'));
			assert.ok(Object.isFrozen(handle.fetch));
			const proto = Object.getPrototypeOf(handle);
			assert.ok(Object.isFrozen(proto));
			assert.isNull(Object.getPrototypeOf(proto));
			assert.isUndefined((handle as any).then);
		}
	});

	test('a handle comes only from the module', () => {
		const ctor = Object.getPrototypeOf(net).constructor;
		assert_throws_type(() => new ctor(Symbol('forged'), null));
	});

	test('`NetError` and its prototype are frozen', () => {
		assert.ok(Object.isFrozen(NetError));
		assert.ok(Object.isFrozen(NetError.prototype));
	});

	test('`narrow` on a borrowed receiver throws `TypeError`', () => {
		const narrow = Object.getPrototypeOf(net).narrow;
		for (const receiver of [{ hosts: ['*'] }, {}, null, undefined]) {
			assert_throws_type(() => narrow.call(receiver, { hosts: ['localhost'] }));
		}
		assert_throws_type(() => narrow.call({ hosts: [] }, { hosts: [] }));
	});

	test('`fetch` destructures, and a receiver can never widen it', async () => {
		const { fetch } = narrowed(['127.0.0.1:1']);
		await assert_rejects_net_error(fetch('http://example.com/'), 'not_granted');
		await assert_rejects_net_error(
			fetch.call({ hosts: ['example.com'] }, 'http://example.com/'),
			'not_granted'
		);
		await assert_rejects_net_error(fetch.call(net, 'http://example.com/'), 'not_granted');
	});

	test('`fetch` assigns to `typeof globalThis.fetch` and rejects rather than throws', async () => {
		const fetch: typeof globalThis.fetch = net.fetch;
		const promise = fetch('http://[bad');
		assert.instanceOf(promise, Promise);
		try {
			await promise;
			assert.fail('expected a TypeError');
		} catch (error) {
			assert.instanceOf(error, TypeError);
		}
	});
});

describe('narrow', () => {
	test('shrinks and never widens', () => {
		const any_port = narrowed(['localhost']);
		const one_port = any_port.narrow({ hosts: ['localhost:3000'] });
		assert.ok(any_port.narrow({ hosts: ['localhost'] }));
		assert.ok(one_port.narrow({ hosts: ['localhost:3000'] }));
		assert_net_error(() => one_port.narrow({ hosts: ['localhost'] }), 'not_granted');
		assert_net_error(() => one_port.narrow({ hosts: ['localhost:3001'] }), 'not_granted');
		assert_net_error(() => any_port.narrow({ hosts: ['example.com'] }), 'not_granted');
		assert_net_error(() => narrowed([]).narrow({ hosts: ['localhost'] }), 'not_granted');
	});

	test('an empty list reaches nothing', async () => {
		await assert_rejects_net_error(narrowed([]).fetch('http://127.0.0.1:1/'), 'not_granted');
	});

	test('reads only own `hosts`, by index, once', () => {
		const inherited = Object.create({ hosts: ['example.com'] });
		assert_throws_type(() => net.narrow(inherited));
		assert_throws_type(() => net.narrow(null as any));
		assert_throws_type(() => net.narrow({ hosts: 'localhost' } as any));
		// a proxy counts reads, its `length` and indexes the only keys touched
		let reads = 0;
		const hosts = new Proxy(['localhost'], {
			get: (target, key, receiver) => {
				if (key === 'length') reads++;
				return Reflect.get(target, key, receiver);
			}
		});
		assert.ok(net.narrow({ hosts }));
		assert.strictEqual(reads, 1);
	});

	test('names the refused host, never a whole URL', async () => {
		const error = await assert_rejects_net_error(
			narrowed(['localhost']).fetch('http://example.com:8080/path?token=secret'),
			'not_granted'
		);
		assert.strictEqual(error.target, 'example.com:8080');
		assert.notInclude(error.message, 'secret');
	});
});

describe('host spelling', () => {
	test('`*` is a `TypeError`, as for `env.narrow`', () => {
		for (const host of ['*', '*.example.com', 'exa*mple.com']) {
			assert_throws_type(() => net.narrow({ hosts: [host] }));
		}
	});

	test('anything but a host is a `TypeError`', () => {
		for (const host of [
			'',
			'.',
			'http://example.com',
			'example.com/path',
			'example.com?q',
			'example.com#f',
			'user@example.com',
			'exa\tmple.com',
			'example.com\x01',
			'exa mple.com',
			'exam\\ple.com',
			'example.com:',
			'example.com:port',
			'example.com:70000',
			'::1',
			'\ud800',
			1 as any,
			null as any,
			Object('example.com')
		]) {
			assert_throws_type(() => net.narrow({ hosts: [host] }));
		}
	});

	test('spellings of one name are one grant', () => {
		const groups = [
			['example.com', 'EXAMPLE.com', 'example.com.', 'ex%61mple.com'],
			['127.0.0.1', '127.1', '0x7f.0.0.1'],
			['[::1]', '[0:0:0:0:0:0:0:1]'],
			['example.com:8080', 'EXAMPLE.com.:8080']
		];
		for (const group of groups) {
			for (const parent of group) {
				for (const child of group) {
					assert.ok(narrowed([parent]).narrow({ hosts: [child] }), `${parent} covers ${child}`);
				}
			}
		}
		// two dots aren't one name
		assert_net_error(
			() => narrowed(['example.com']).narrow({ hosts: ['example.com..'] }),
			'not_granted'
		);
		assert_net_error(() => narrowed(['localhost']).narrow({ hosts: ['127.0.0.1'] }), 'not_granted');
	});

	test('a default port is spelled and compared as a number', async () => {
		const grant_80 = narrowed(['127.0.0.1:80']);
		const grant_443 = narrowed(['127.0.0.1:443']);
		assert.ok(grant_80.narrow({ hosts: ['127.0.0.1:80'] }));
		assert_net_error(() => grant_80.narrow({ hosts: ['127.0.0.1:443'] }), 'not_granted');
		await assert_rejects_net_error(grant_443.fetch('http://127.0.0.1/'), 'not_granted');
		await assert_rejects_net_error(grant_80.fetch('https://127.0.0.1/'), 'not_granted');
		await assert_rejects_net_error(grant_80.fetch('http://127.0.0.1:81/'), 'not_granted');
	});

	test('a request host is compared with its trailing dot dropped', async () => {
		const handle = narrowed(['example.com']);
		await assert_rejects_net_error(handle.fetch('http://example.com../'), 'not_granted');
		await assert_rejects_net_error(handle.fetch('http://other.com./'), 'not_granted');
	});
});

describe('schemes', () => {
	test('only http and https, on every handle', async () => {
		for (const url of ['data:,x', 'blob:http://x/y', 'file:///etc/passwd', 'ftp://example.com/']) {
			for (const handle of [net, narrowed(['example.com'])]) {
				await assert_rejects_net_error(handle.fetch(url), 'not_granted');
			}
		}
	});
});

describe('granted URLs reach the send', () => {
	// the module copies `fetch` at load, so a fresh copy under a stub sends nothing anywhere
	const load_with_stub = async (): Promise<{ net: Net; sent: Array<string> }> => {
		const sent: Array<string> = [];
		vi.resetModules();
		vi.stubGlobal('fetch', async (request: Request) => {
			sent.push(request.url);
			return new Response('stubbed');
		});
		const { net: fresh } = await import('#lib/net.ts');
		return { net: fresh, sent };
	};

	afterEach(() => {
		vi.unstubAllGlobals();
		vi.resetModules();
	});

	test('a host grant matches its normalized spellings, any port', async () => {
		const { net: fresh, sent } = await load_with_stub();
		const handle = fresh.narrow({ hosts: ['EXAMPLE.com'] });
		await handle.fetch('http://example.com/a');
		await handle.fetch('https://example.com.:8443/b');
		await handle.fetch('http://EXAMPLE.COM/c');
		assert.deepEqual(sent, [
			'http://example.com/a',
			'https://example.com.:8443/b',
			'http://example.com/c'
		]);
	});

	test('a port grant matches that port, a default spelled or not', async () => {
		const { net: fresh, sent } = await load_with_stub();
		const https_grant = fresh.narrow({ hosts: ['example.com:443'] });
		await https_grant.fetch('https://example.com/a');
		await https_grant.fetch('https://example.com:443/b');
		await assert_rejects_net_error(https_grant.fetch('http://example.com/c'), 'not_granted');
		await assert_rejects_net_error(https_grant.fetch('https://example.com:8443/d'), 'not_granted');
		const http_grant = fresh.narrow({ hosts: ['example.com:80'] });
		await http_grant.fetch('http://example.com/e');
		await assert_rejects_net_error(http_grant.fetch('https://example.com/f'), 'not_granted');
		assert.deepEqual(sent, [
			'https://example.com/a',
			'https://example.com/b',
			'http://example.com/e'
		]);
	});

	test('a granted IP and IPv6 literal match', async () => {
		const { net: fresh, sent } = await load_with_stub();
		const handle = fresh.narrow({ hosts: ['10.0.0.1', '[::1]:9000'] });
		await handle.fetch('http://10.0.0.1:1234/a');
		await handle.fetch('http://[::1]:9000/b');
		await assert_rejects_net_error(handle.fetch('http://[::1]:9001/c'), 'not_granted');
		assert.deepEqual(sent, ['http://10.0.0.1:1234/a', 'http://[::1]:9000/b']);
	});

	test('the root reaches any http(s) host', async () => {
		const { net: fresh, sent } = await load_with_stub();
		await fresh.fetch('https://anything.example/');
		assert.deepEqual(sent, ['https://anything.example/']);
	});
});
