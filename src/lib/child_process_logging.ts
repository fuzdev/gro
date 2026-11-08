import { strip_end } from '@fuzdev/fuz_util/string.ts';

import type { SpawnChild } from './spawn.ts';

/**
 * Maps a child's piped output through a transform function to this process's stdout and stderr,
 * chunk by chunk, decoding each stream as UTF-8 across chunk boundaries.
 * @returns a promise that resolves once both streams have ended and been written
 */
export const map_child_process_output = async (
	child: SpawnChild,
	transform: (data: string) => string
): Promise<void> => {
	await Promise.all([
		pipe_mapped(child.stdout, process.stdout, transform),
		pipe_mapped(child.stderr, process.stderr, transform)
	]);
};

const pipe_mapped = async (
	source: AsyncIterable<Uint8Array> | null,
	target: NodeJS.WriteStream,
	transform: (data: string) => string
): Promise<void> => {
	if (source === null) return;
	const decoder = new TextDecoder('utf-8', { ignoreBOM: true });
	for await (const chunk of source) {
		target.write(transform(decoder.decode(chunk, { stream: true })));
	}
	const rest = decoder.decode();
	if (rest) target.write(transform(rest));
};

/**
 * Configures process output handling with path replacements while preserving ANSI colors.
 * @returns a promise that resolves once the child's output has ended and been written
 */
export const configure_colored_output_with_path_replacement = (
	child: SpawnChild,
	replacement: string = '.',
	cwd: string = process.cwd()
): Promise<void> => {
	// Escape special characters in the cwd for regexp safety
	const cwd_escaped = strip_end(cwd, '/').replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
	const cwd_reg_exp = new RegExp(cwd_escaped, 'g');

	// Use the generic mapper with a path replacement transform
	return map_child_process_output(child, (data) => data.replace(cwd_reg_exp, replacement));
};
