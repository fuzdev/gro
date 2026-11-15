import { describe, test, assert } from 'vitest';

import { env } from '#lib/env.ts';
import { fs } from '#lib/fs.ts';
import { net } from '#lib/net.ts';
import { spawn } from '#lib/spawn.ts';
import { stderr } from '#lib/stderr.ts';
import { stdout } from '#lib/stdout.ts';
import { watch } from '#lib/watch.ts';

/**
 * Asserts that every function `value` holds as an own property, or that a prototype on its chain
 * holds, a method's or an accessor's, is frozen, so a write to a property of one throws, as the
 * module's code is strict. Returns how many it checked.
 */
const assert_methods_frozen = (value: object, label: string): number => {
	let checked = 0;
	for (let o: object | null = value; o !== null; o = Object.getPrototypeOf(o)) {
		for (const key of Reflect.ownKeys(o)) {
			const { value: method, get, set } = Object.getOwnPropertyDescriptor(o, key)!;
			for (const fn of [method, get, set]) {
				if (typeof fn !== 'function') continue;
				const name = `${label} ${String(key)}`;
				assert.ok(Object.isFrozen(fn), `${name} is frozen`);
				assert.throws(() => {
					(fn as unknown as Record<string, unknown>).stash = 'x';
				}, TypeError);
				assert.ok(!('stash' in fn), `${name} took no property`);
				// a generator method's iterators inherit from its own `prototype`, shared by every holder
				const own_prototype = Object.getOwnPropertyDescriptor(fn, 'prototype')?.value;
				if (typeof own_prototype === 'object' && own_prototype !== null) {
					assert.ok(Object.isFrozen(own_prototype), `${name}.prototype is frozen`);
					assert.throws(() => {
						(own_prototype as Record<string, unknown>).next = () => 'forged';
					}, TypeError);
				}
				checked++;
			}
		}
	}
	return checked;
};

describe("the host modules' handles", () => {
	test("each handle's methods, and each root's functions, are frozen", async () => {
		const handles: Array<[string, object]> = [
			['env', env],
			['env.narrow', env.narrow(['PATH'])],
			['fs', fs],
			['fs.project', fs.project],
			['fs.project.readonly', fs.project.readonly()],
			['net', net],
			['net.narrow', net.narrow({ hosts: ['example.com'] })],
			['spawn', spawn],
			['spawn.narrow', spawn.narrow(['cat'])],
			['stdout', stdout],
			['stderr', stderr],
			['watch', watch]
		];
		await using temp = await fs.temp_dir();
		handles.push(['fs.temp_dir', temp]);

		const watcher = await watch.dir(temp, { recursive: false });
		handles.push(['watcher', watcher], ['watcher iterator', watcher[Symbol.asyncIterator]()]);

		const cat = spawn.program('cat');
		handles.push(['program', cat]);
		const child = await cat.start([], { stdio: 'pipe' });
		handles.push(
			['child', child],
			['child.stdin', child.stdin!],
			['child.stdout', child.stdout!],
			['child.stdout iterator', child.stdout![Symbol.asyncIterator]()]
		);
		try {
			for (const [label, handle] of handles) {
				assert.ok(assert_methods_frozen(handle, label) > 0, `${label} has methods`);
			}
			// `list` is a generator method, whose listings inherit from its frozen `prototype`
			const listing = fs.project.list();
			assert.strictEqual(
				Object.getPrototypeOf(listing),
				(fs.project.list as unknown as { prototype: object }).prototype
			);
			assert.ok(Object.isFrozen(Object.getPrototypeOf(listing)));
		} finally {
			await child.stdin!.close();
			await child.closed;
			await watcher.close();
		}
	});
});
