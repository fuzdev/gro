import { TaskError } from '../task_error.ts';

/*

A builtin that forwards a section to its tool refuses the tool's help and version flags there,
which make the tool print in place of running and exit 0, so the step would pass without running.
It's a guardrail against slips, not a complete check: only the exact forms are refused,
so a deliberate spelling (`--help=x`, a bundle like `-hv`, tsc's `-?`) still reaches the tool.
It lives under `internal/` because it's no API: each builtin calls it where it reads its section.

*/

/**
 * Refuses `--help` and `-h` in `forwarded_argv`, and with `version`, `--version` and `-v`,
 * each as a whole arg.
 * @param tool - the tool the args are forwarded to, named in the message
 * @param forwarded_argv - the args forwarded to it, from its section
 * @param version - whether the tool reads a version flag in place of running;
 *   Vitest and Vite ignore one after the command Gro gives them, so they pass `false`
 * @throws `TaskError` naming the flag, the tool, and running the tool directly
 */
export const assert_no_forwarded_info_flag = (
	tool: string,
	forwarded_argv: ReadonlyArray<string>,
	version: boolean
): void => {
	for (const flag of forwarded_argv) {
		const kind =
			flag === '--help' || flag === '-h'
				? 'help'
				: version && (flag === '--version' || flag === '-v')
					? 'version'
					: null;
		if (kind === null) continue;
		throw new TaskError(
			`\`${flag}\` forwarded to ${tool} asks it for its ${kind}, not a run, ` +
				`so the step could pass without one - run \`${tool} ${flag}\` directly`
		);
	}
};
