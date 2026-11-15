import { describe, test, assert } from 'vitest';

import { FsError } from '#lib/fs_error.ts';
import { format_error_causes, format_error_detail, MAX_ERROR_CAUSES } from '#lib/error_detail.ts';

describe('format_error_detail', () => {
	test('prints the message alone for an error with no cause', () => {
		assert.strictEqual(
			format_error_detail(new FsError('not_found', '/p/a.txt')),
			'not_found: /p/a.txt'
		);
		assert.isNull(format_error_causes(new FsError('not_found', '/p/a.txt')));
	});

	test('nests each cause in parentheses inside the one before', () => {
		const os = new Error("ENOENT: no such file or directory, open '/p/real/a.txt'");
		const inner = new FsError('not_found', '/p/real/a.txt', { cause: os });
		const outer = new FsError('not_found', '/p/a.txt', { cause: inner });
		assert.strictEqual(
			format_error_detail(outer),
			"not_found: /p/a.txt (not_found: /p/real/a.txt (ENOENT: no such file or directory, open '/p/real/a.txt'))"
		);
		assert.strictEqual(
			format_error_causes(outer),
			"not_found: /p/real/a.txt (ENOENT: no such file or directory, open '/p/real/a.txt')"
		);
	});

	test('prints a string cause, skips an empty message, and stops at any other cause', () => {
		assert.strictEqual(format_error_detail(new Error('a', { cause: 'b' })), 'a (b)');
		assert.strictEqual(
			format_error_detail(new Error('a', { cause: new Error('', { cause: 'c' }) })),
			'a (c)'
		);
		assert.strictEqual(format_error_detail(new Error('a', { cause: { message: 'no' } })), 'a');
		assert.strictEqual(format_error_detail(new Error('a', { cause: 1 })), 'a');
	});

	test('prints a thrown value that is no error', () => {
		assert.strictEqual(format_error_detail('a string'), 'a string');
		assert.strictEqual(format_error_detail(42), '42');
		assert.strictEqual(format_error_detail(undefined), 'undefined');
		assert.strictEqual(format_error_detail(null), '[null]');
		assert.strictEqual(format_error_detail({ message: 'no' }), '[object]');
		assert.strictEqual(format_error_detail(Symbol('s\n')), 'Symbol(s\\u{a})');
	});

	test('prints a symbol though `Symbol.prototype.toString` is patched to throw', () => {
		// patched as a program could, through a reference to the prototype
		const prototype: object = Symbol.prototype;
		const original = Object.getOwnPropertyDescriptor(prototype, 'toString')!;
		Object.defineProperty(prototype, 'toString', {
			...original,
			value: () => {
				throw new Error('patched');
			}
		});
		try {
			assert.strictEqual(format_error_detail(Symbol('s')), 'Symbol(s)');
		} finally {
			Object.defineProperty(prototype, 'toString', original);
		}
	});

	test('ends a cycle at the cause already printed, one that skips the first error included', () => {
		const a = new Error('a');
		const b = new Error('b', { cause: a });
		Object.defineProperty(a, 'cause', { value: b });
		assert.strictEqual(format_error_detail(a), 'a (b)');
		// a -> b -> c -> b: the repeat isn't the error printed first
		const c = new Error('c');
		const b2 = new Error('b', { cause: c });
		Object.defineProperty(c, 'cause', { value: b2 });
		assert.strictEqual(format_error_detail(new Error('a', { cause: b2 })), 'a (b (c))');
	});

	test('prints at most `MAX_ERROR_CAUSES` causes, then `…`', () => {
		let error = new Error('last');
		for (let i = 20; i > 0; i--) error = new Error(String(i), { cause: error });
		const detail = format_error_detail(new Error('top', { cause: error }));
		const shown = Array.from({ length: MAX_ERROR_CAUSES }, (_, i) => String(i + 1));
		assert.strictEqual(detail, `top (${shown.join(' (')} (…${')'.repeat(MAX_ERROR_CAUSES + 1)}`);
	});

	test('never throws: a getter that throws or a proxy ends the chain there', () => {
		const throwing_cause = new Error('a');
		Object.defineProperty(throwing_cause, 'cause', {
			get() {
				throw new Error('trap');
			}
		});
		assert.strictEqual(format_error_detail(throwing_cause), 'a');
		const throwing_message = new Error('x');
		Object.defineProperty(throwing_message, 'message', {
			get() {
				throw new Error('trap');
			}
		});
		assert.strictEqual(format_error_detail(new Error('a', { cause: throwing_message })), 'a');
		const revocable = Proxy.revocable(new Error('b'), {});
		revocable.revoke();
		assert.strictEqual(format_error_detail(new Error('a', { cause: revocable.proxy })), 'a');
		assert.strictEqual(format_error_detail(revocable.proxy), '[object]');
	});

	test('escapes control and bidirectional formatting characters', () => {
		assert.strictEqual(
			format_error_detail(new Error('a\nb\u001b[2K', { cause: 'c‮d\u0085' })),
			'a\\u{a}b\\u{1b}[2K (c\\u{202e}d\\u{85})'
		);
	});
});
