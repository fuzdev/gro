import { strip_end } from '@fuzdev/fuz_util/string.ts';

import type { ProcessOutput } from './stdout.ts';
import type { SpawnChild } from './spawn.ts';

/**
 * Where a child's mapped output goes, like the `stdout` and `stderr` host modules.
 */
export interface ChildOutputTargets {
	stdout: ProcessOutput;
	stderr: ProcessOutput;
}

/**
 * Maps a child's piped output through a transform function to `targets`,
 * chunk by chunk, decoding each stream as UTF-8 across chunk boundaries.
 * Each write is awaited, so a slow reader paces the child.
 * @param targets - where the output goes, `stdout` and `stderr` at a root
 * @returns a promise that resolves once both streams have ended and been written
 */
export const map_child_process_output = async (
	child: SpawnChild,
	transform: (data: string) => string,
	targets: ChildOutputTargets
): Promise<void> => {
	await Promise.all([
		pipe_mapped(child.stdout, targets.stdout, transform),
		pipe_mapped(child.stderr, targets.stderr, transform)
	]);
};

const pipe_mapped = async (
	source: AsyncIterable<Uint8Array> | null,
	target: ProcessOutput,
	transform: (data: string) => string
): Promise<void> => {
	if (source === null) return;
	const decoder = new TextDecoder('utf-8', { ignoreBOM: true });
	for await (const chunk of source) {
		// `false` means the reader left: keep reading so the child never blocks on a full pipe
		await target.write(transform(decoder.decode(chunk, { stream: true })));
	}
	const rest = decoder.decode();
	if (rest) await target.write(transform(rest));
};

/**
 * Configures process output handling with path replacements while preserving ANSI colors.
 * @param replacement - what `cwd` becomes in the output
 * @param cwd - the absolute path to replace
 * @param targets - where the output goes
 * @returns a promise that resolves once the child's output has ended and been written
 */
export const configure_colored_output_with_path_replacement = (
	child: SpawnChild,
	replacement: string,
	cwd: string,
	targets: ChildOutputTargets
): Promise<void> => {
	// Escape special characters in the cwd for regexp safety
	const cwd_escaped = strip_end(cwd, '/').replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
	const cwd_reg_exp = new RegExp(cwd_escaped, 'g');

	// Use the generic mapper with a path replacement transform
	return map_child_process_output(child, (data) => data.replace(cwd_reg_exp, replacement), targets);
};
