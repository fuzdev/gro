import { describe, test, assert } from 'vitest';
import { runInNewContext } from 'node:vm';

import { spawn } from '#lib/spawn.ts';

const cat = () => spawn.program('cat');

describe('`stdin` data, what counts as a `Uint8Array`', () => {
	test('a `Uint8Array` from another realm is data', async () => {
		const stdin = runInNewContext('new Uint8Array([104, 105])') as Uint8Array;
		assert.strictEqual((await cat().out([], { stdin })).stdout, 'hi');
	});

	test('a `Buffer` is copied, not viewed', async () => {
		const stdin = Buffer.from('original');
		const call = cat().out([], { stdin });
		stdin.fill(0x7a);
		assert.strictEqual((await call).stdout, 'original');
	});

	test('a subclass gets no say in what is sent', async () => {
		class Lying extends Uint8Array {
			override get length(): number {
				return 1;
			}
			override slice(): this {
				throw Error('slice called');
			}
		}
		const stdin = new Lying(new TextEncoder().encode('all of it'));
		assert.strictEqual((await cat().out([], { stdin })).stdout, 'all of it');
	});
});
