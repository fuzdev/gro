import type {ChildProcess} from 'node:child_process';
import {strip_end} from '@fuzdev/fuz_util/string.ts';

/**
 * Maps child process output through a transform function.
 */
export const map_child_process_output = (
	child_process: ChildProcess,
	transform: (data: string) => string,
): void => {
	if (child_process.stdout) {
		child_process.stdout.on('data', (data) => {
			process.stdout.write(transform(data.toString()));
		});
	}

	if (child_process.stderr) {
		child_process.stderr.on('data', (data) => {
			process.stderr.write(transform(data.toString()));
		});
	}
};

/**
 * Configures process output handling with path replacements while preserving ANSI colors.
 */
export const configure_colored_output_with_path_replacement = (
	child_process: ChildProcess,
	replacement: string = '.',
	cwd: string = process.cwd(),
): void => {
	// Escape special characters in the cwd for regexp safety
	const cwd_escaped = strip_end(cwd, '/').replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
	const cwd_reg_exp = new RegExp(cwd_escaped, 'g');

	// Use the generic mapper with a path replacement transform
	map_child_process_output(child_process, (data) => data.replace(cwd_reg_exp, replacement));
};
