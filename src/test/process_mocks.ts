import { vi } from 'vitest';

import type { Process, ProcessOutput } from '#lib/process.ts';

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

const mock_cli_args: { current: ReadonlyArray<string> } = { current: [] };

/**
 * Sets the args a mocked `process.args` reads, the CLI's args after its own path.
 */
export const set_mock_process_args = (args: ReadonlyArray<string>): void => {
	mock_cli_args.current = args;
};

/**
 * A mock of `#lib/process.ts` for tests of a root that reads `process.args`,
 * every other field the real module's, so only the snapshot the module can't replace differs.
 */
export const create_mock_process_module = async (): Promise<typeof import('#lib/process.ts')> => {
	const actual = await vi.importActual<typeof import('#lib/process.ts')>('#lib/process.ts');
	const process: Process = {
		env: actual.process.env,
		get args() {
			return mock_cli_args.current;
		},
		cwd: () => actual.process.cwd(),
		exit: (code) => actual.process.exit(code),
		set_exit_code: (code) => actual.process.set_exit_code(code),
		stdin: actual.process.stdin,
		stdout: actual.process.stdout,
		stderr: actual.process.stderr,
		pid: actual.process.pid,
		runtime_version: actual.process.runtime_version
	};
	return { ...actual, process };
};
