/**
 * The Node CLI's listeners on the global `process`: its output streams' failures and the
 * uncaught errors, which no host module has a listener for.
 *
 * Node-only. The CLI's entry, `invoke.ts`, installs them once, before anything runs.
 *
 * @module
 */

/**
 * Listens for the process's uncaught errors and its output streams' failures.
 *
 * A reader of stdout or stderr that left, like `head` in `gro foo | head`, only loses the writes
 * after it left: the run finishes with its own exit code, so `gro foo | head` exits `0`
 * only when `foo` passed, and a run that never ends on its own runs until signalled.
 * Any other stream failure is rethrown, as with no listener, unless the host is already exiting
 * on an uncaught error, whose own writes to the same failed output would otherwise never end.
 * @param on_uncaught - handles an uncaught error and its origin, and ends the process
 */
export const listen_to_node_process = (
	on_uncaught: (error: Error, origin: string) => void
): void => {
	const host_process = globalThis.process;
	let exiting = false;
	const on_output_error = (error: NodeJS.ErrnoException): void => {
		if (error.code === 'EPIPE' || exiting) return;
		throw error;
	};
	host_process.stdout.on('error', on_output_error);
	host_process.stderr.on('error', on_output_error);
	host_process.on('uncaughtException', (error, origin) => {
		exiting = true;
		on_uncaught(error, origin);
	});
};
