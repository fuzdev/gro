import type { ProcessOutput } from '#lib/stdout.ts';

/**
 * A `ProcessOutput` recording its writes, what a library's output parameter takes in a test.
 */
export interface MockProcessOutput extends ProcessOutput {
	written: Array<string | Uint8Array>;
}

/**
 * Creates a mock output whose writes resolve `true`, recording each chunk.
 */
export const create_mock_process_output = (): MockProcessOutput => {
	const written: Array<string | Uint8Array> = [];
	return {
		is_terminal: false,
		written,
		write: async (data) => {
			written.push(data);
			return true;
		}
	};
};
