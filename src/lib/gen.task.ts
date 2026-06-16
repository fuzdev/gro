import {styleText as st} from 'node:util';
import {print_ms, print_error} from '@fuzdev/fuz_util/print.ts';
import {plural} from '@fuzdev/fuz_util/string.ts';
import {z} from 'zod';

import {TaskError, type Task} from './task.ts';
import {run_gen} from './run_gen.ts';
import {RawInputPath, to_input_paths} from './input_path.ts';
import {format_file} from './format_file.ts';
import {print_path} from './paths.ts';
import {log_error_reasons} from './task_logging.ts';
import {
	write_gen_results,
	analyze_gen_results,
	find_genfiles,
	load_genfiles,
	type AnalyzedGenResult,
	type GenResults,
} from './gen.ts';
import {SOURCE_DIRNAME} from './constants.ts';

/** @nodocs */
export const Args = z.strictObject({
	_: z.array(RawInputPath).meta({description: 'input paths to generate'}).default([SOURCE_DIRNAME]),
	root_dirs: z
		.array(z.string())
		.meta({description: 'root directories to resolve input paths against'}) // TODO `PathId` schema
		.default([process.cwd()]),
	check: z
		.boolean()
		.meta({description: 'exit with a nonzero code if any files need to be generated'})
		.default(false),
});
export type Args = z.infer<typeof Args>;

/** @nodocs */
export const task: Task<Args> = {
	summary: 'run code generation scripts',
	Args,
	run: async ({args, filer, log, timings, config, invoke_task}): Promise<void> => {
		const {_: raw_input_paths, root_dirs, check} = args;

		const input_paths = to_input_paths(raw_input_paths);

		// load all of the gen modules
		const found = await find_genfiles(input_paths, root_dirs, config, timings);
		if (!found.ok) {
			if (found.type === 'input_directories_with_no_files') {
				// TODO maybe let this error like the normal case, but only call `gro gen` if we find gen files? problem is the args would need to be hoisted to callers like `gro sync`
				log.info('no gen modules found in ' + input_paths.join(', '));
				return;
			} else {
				log_error_reasons(log, found.reasons);
				throw new TaskError('Failed to find gen modules.');
			}
		}
		const found_genfiles = found.value;
		log.info(
			'gen files',
			found_genfiles.resolved_input_files.map((f) => f.id),
		);
		const loaded = await load_genfiles(found_genfiles, timings);
		if (!loaded.ok) {
			log_error_reasons(log, loaded.reasons);
			throw new TaskError('Failed to load gen modules.');
		}
		const loaded_genfiles = loaded.value;

		// run `gen` on each of the modules
		const timing_to_generate_code = timings.start('generate code'); // TODO this ignores `gen_results.elapsed` - should it return `Timings` instead?
		const gen_results = await run_gen(
			loaded_genfiles.modules,
			config,
			filer,
			log,
			timings,
			invoke_task,
			format_file,
		);
		timing_to_generate_code();

		const fail_count = gen_results.failures.length;
		const analyzed_gen_results = await analyze_gen_results(gen_results);
		if (check) {
			// check if any files changed, and if so, throw errors,
			// but if there are gen failures, skip the check and defer to their errors
			if (!fail_count) {
				log.info('checking generated files for changes');
				const timing_to_check_results = timings.start('check results for changes');
				timing_to_check_results();

				let has_unexpected_changes = false;
				for (const analyzed of analyzed_gen_results) {
					if (!analyzed.has_changed) continue;
					has_unexpected_changes = true;
					log.error(
						st(
							'red',
							`Generated file ${print_path(analyzed.file.id)} via ${print_path(
								analyzed.file.origin_id,
							)} ${analyzed.is_new ? 'is new' : 'has changed'}.`,
						),
					);
				}
				if (has_unexpected_changes) {
					throw new TaskError(
						'Failed gen check. Some generated files have unexpectedly changed.' +
							' Run `gro gen` and try again.',
					);
				}
				log.info('check passed, no files have changed');
			}
		} else {
			const timing_to_output_results = timings.start('output results');
			await write_gen_results(gen_results, analyzed_gen_results, log);
			timing_to_output_results();
		}

		// collect and format output with summary
		const output_lines = collect_output_lines(gen_results, analyzed_gen_results);
		const new_count = analyzed_gen_results.filter((r) => r.is_new).length;
		const changed_count = analyzed_gen_results.filter((r) => r.has_changed && !r.is_new).length;
		const unchanged_count = analyzed_gen_results.filter((r) => !r.is_new && !r.has_changed).length;
		const error_count = gen_results.failures.length;

		log.info(
			format_gen_output(output_lines) +
				`\n\n\t${new_count} ${st(new_count > 0 ? 'green' : 'gray', 'new')}, ${changed_count} ${st(changed_count > 0 ? 'cyan' : 'gray', 'changed')}, ${unchanged_count} ${st('gray', 'unchanged')}${error_count ? `, ${error_count} ${st('red', 'error' + plural(error_count))}` : ''} from ${gen_results.input_count} input file${plural(gen_results.input_count)}`,
		);

		if (fail_count) {
			for (const result of gen_results.failures) {
				log.error(result.reason, '\n', print_error(result.error));
			}
			throw new TaskError(`Failed to generate ${fail_count} file${plural(fail_count)}.`);
		}
	},
};

interface GenStatus {
	symbol: string;
	color: Parameters<typeof st>[0];
	text: string;
}

const format_gen_status = (analyzed: AnalyzedGenResult | undefined): GenStatus => {
	if (!analyzed) return {symbol: '?', color: 'gray', text: 'unknown'};
	if (analyzed.is_new) return {symbol: '●', color: 'green', text: 'new'};
	if (analyzed.has_changed) return {symbol: '◐', color: 'cyan', text: 'changed'};
	return {symbol: '○', color: 'gray', text: 'unchanged'};
};

interface OutputLine {
	status: GenStatus;
	elapsed: string;
	source: string;
	target: string;
	is_error: boolean;
}

const collect_output_lines = (
	gen_results: GenResults,
	analyzed_gen_results: Array<AnalyzedGenResult>,
): Array<OutputLine> => {
	const output_lines: Array<OutputLine> = [];

	for (const result of gen_results.results) {
		if (result.ok) {
			for (const file of result.files) {
				const analyzed = analyzed_gen_results.find((a) => a.file.id === file.id);
				output_lines.push({
					status: format_gen_status(analyzed),
					elapsed: print_ms(result.elapsed),
					source: print_path(result.id),
					target: print_path(file.id),
					is_error: false,
				});
			}
		} else {
			output_lines.push({
				status: {symbol: '🞩', color: 'red', text: 'error'},
				elapsed: print_ms(result.elapsed),
				source: print_path(result.id),
				target: st('red', result.error.stack || result.error.message || 'error'),
				is_error: true,
			});
		}
	}

	return output_lines;
};

const format_gen_output = (output_lines: Array<OutputLine>): string => {
	// calculate column widths for alignment
	const max_elapsed_length = Math.max(...output_lines.map((l) => l.elapsed.length));
	const max_source_length = Math.max(...output_lines.map((l) => l.source.length));

	// format the output lines
	let log_result = 'gen results:';
	for (const line of output_lines) {
		const elapsed_text = line.elapsed.padStart(max_elapsed_length);
		const source_text = line.source.padEnd(max_source_length);
		log_result += `\n\t${st(line.status.color, line.status.symbol)}  ${elapsed_text}  ${source_text} → ${line.target}`;
	}
	return log_result;
};
