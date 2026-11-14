import { describe, test, assert, vi, afterEach } from 'vitest';
import { wait } from '@fuzdev/fuz_util/async.ts';

import {
	configure_colored_output_with_path_replacement,
	map_child_process_output,
	type ChildOutputTargets
} from '#lib/child_process_logging.ts';

import { create_mock_process_output } from './process_output_mocks.ts';
import { create_mock_child, create_mock_stream } from './spawn_mocks.ts';

const bytes = (...values: Array<number>): Uint8Array => new Uint8Array(values);

/**
 * Output targets recording what's written to stdout and stderr, as text.
 */
const capture_output = (): {
	out: Array<string>;
	err: Array<string>;
	targets: ChildOutputTargets;
} => {
	const stdout = create_mock_process_output();
	const stderr = create_mock_process_output();
	return {
		out: stdout.written as Array<string>,
		err: stderr.written as Array<string>,
		targets: { stdout, stderr }
	};
};

afterEach(() => {
	vi.restoreAllMocks();
});

describe('map_child_process_output', () => {
	test('a multi-byte character split across chunks decodes whole, per stream', async () => {
		const { out, err, targets } = capture_output();
		const stdout = create_mock_stream();
		const stderr = create_mock_stream();
		const written = map_child_process_output(
			create_mock_child({ stdout: stdout.stream, stderr: stderr.stream }),
			(data) => data,
			targets
		);
		// the euro sign's three bytes and a four-byte emoji, interleaved across the streams
		stdout.push(bytes(0x61, 0xe2));
		stderr.push(bytes(0xf0, 0x9f));
		stdout.push(bytes(0x82));
		stderr.push(bytes(0x99));
		stdout.push(bytes(0xac, 0x62));
		stderr.push(bytes(0x82));
		stdout.end();
		stderr.end();
		await written;
		assert.strictEqual(out.join(''), 'a€b');
		assert.strictEqual(err.join(''), '🙂');
	});

	test('a truncated character at the end is flushed through the transform', async () => {
		const { out, targets } = capture_output();
		const stdout = create_mock_stream();
		const written = map_child_process_output(
			create_mock_child({ stdout: stdout.stream }),
			(data) => `[${data}]`,
			targets
		);
		stdout.push(bytes(0x61, 0xe2));
		stdout.end();
		await written;
		assert.strictEqual(out.join(''), '[a][�]');
	});

	test('resolves only after both streams have ended and been written', async () => {
		const { out, err, targets } = capture_output();
		const stdout = create_mock_stream();
		const stderr = create_mock_stream();
		let settled = false;
		const written = map_child_process_output(
			create_mock_child({ stdout: stdout.stream, stderr: stderr.stream }),
			(data) => data,
			targets
		).then(() => {
			settled = true;
		});

		stdout.push(bytes(0x61));
		await wait(0);
		assert.strictEqual(out.join(''), 'a');
		assert.ok(!settled);

		stdout.end();
		await wait(0);
		assert.ok(!settled, 'stderr is still open');

		stderr.push(bytes(0x62));
		await wait(0);
		assert.strictEqual(err.join(''), 'b');
		assert.ok(!settled, 'stderr has not ended');

		stderr.end();
		await written;
		assert.ok(settled);
	});

	test('a child with no piped output resolves', async () => {
		await map_child_process_output(create_mock_child(), (data) => data, capture_output().targets);
	});
});

describe('configure_colored_output_with_path_replacement', () => {
	test('replaces the cwd, trailing slash and regexp characters included, keeping colors', async () => {
		const { out, targets } = capture_output();
		const stdout = create_mock_stream();
		const written = configure_colored_output_with_path_replacement(
			create_mock_child({ stdout: stdout.stream }),
			'.',
			'/a.b/c/',
			targets
		);
		stdout.push(new TextEncoder().encode('\x1b[31m/a.b/c/x.ts\x1b[0m /axb/c/y.ts'));
		stdout.end();
		await written;
		assert.strictEqual(out.join(''), '\x1b[31m./x.ts\x1b[0m /axb/c/y.ts');
	});
});
