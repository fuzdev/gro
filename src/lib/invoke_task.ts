import type {Args} from '@fuzdev/fuz_util/args.js';
import {Logger} from '@fuzdev/fuz_util/log.js';
import {print_ms, print_timings} from '@fuzdev/fuz_util/print.js';
import {create_stopwatch, Timings} from '@fuzdev/fuz_util/timings.js';
import {styleText as st} from 'node:util';

import {to_forwarded_args} from './args.ts';
import {run_task} from './run_task.ts';
import {to_input_path, RawInputPath} from './input_path.ts';
import {find_tasks, load_tasks, SilentError} from './task.ts';
import {package_json_load_for_gro} from './package_json.ts';
import {log_tasks, log_error_reasons} from './task_logging.ts';
import type {GroConfig} from './gro_config.ts';
import {Filer} from './filer.ts';

/**
 * Invokes Gro tasks by name using the filesystem as the source.
 *
 * When a task is invoked,
 * Gro first searches for tasks in the current working directory.
 * and falls back to searching Gro's directory, if the two are different.
 * See `src/lib/input_path.ts` for info about what "task_name" can refer to.
 * If it matches a directory, all of the tasks within it are logged,
 * both in the current working directory and Gro.
 *
 * This code is particularly hairy because
 * we're accepting a wide range of user input
 * and trying to do the right thing.
 * Precise error messages are especially difficult and
 * there are some subtle differences in the complex logical branches.
 * The comments describe each condition.
 *
 * @param task_name - the name of the task to invoke
 * @param args - the CLI args to pass to the task
 * @param config - the Gro configuration
 * @param initial_timings - the timings to use for the top-level task, `null` for composed tasks
 */
export const invoke_task = async (
	task_name: RawInputPath,
	args: Args | undefined,
	config: GroConfig,
	initial_filer?: Filer,
	initial_timings?: Timings | null,
	parent_log?: Logger,
): Promise<void> => {
	// Create child logger if parent exists, otherwise root logger
	const log_label = task_name || 'gro';
	const log = parent_log ? parent_log.child(log_label) : new Logger(log_label);
	log.info('invoking', task_name ? st('cyan', task_name) : 'gro');

	// track if we created the filer
	const owns_filer = !initial_filer;
	const filer = initial_filer ?? new Filer({log: log.child('filer'), ...config.filer_options});

	const owns_timings = !initial_timings;
	const timings = initial_timings ?? new Timings();

	const total_timing = create_stopwatch();
	const finish = async () => {
		// cleanup filer only if we created it and it was initialized
		if (owns_filer && filer.inited) {
			await filer.close();
		}

		if (owns_timings) return; // kinda weird, print timings only for the top-level task
		print_timings(timings, log);
		log.info(`🕒 ${print_ms(total_timing())}`);
	};

	// Check if the caller just wants to see the version.
	if (!task_name && (args?.version || args?.v)) {
		const gro_package_json = await package_json_load_for_gro();
		log.info(`${st('gray', 'v')}${st('cyan', gro_package_json.version ?? '[no version]')}`);
		await finish();
		return;
	}

	// Resolve the input path for the provided task name.
	const input_path = to_input_path(task_name);

	const {task_root_dirs} = config;

	// Find the task or directory specified by the `input_path`.
	// Fall back to searching the Gro directory as well.
	const found = await find_tasks([input_path], task_root_dirs, config);
	if (!found.ok) {
		log_error_reasons(log, found.reasons);
		throw new SilentError();
	}

	// Found a match either in the current working directory or Gro's directory.
	const found_tasks = found.value;
	const {resolved_input_files} = found_tasks;

	// Load the task module.
	const loaded = await load_tasks(found_tasks);
	if (!loaded.ok) {
		log_error_reasons(log, loaded.reasons);
		throw new SilentError();
	}
	const loaded_tasks = loaded.value;

	if (
		resolved_input_files.length > 1 ||
		resolved_input_files[0]!.resolved_input_path.is_directory
	) {
		// The input path matches a directory. Log the tasks but don't run them.
		log_tasks(log, loaded_tasks);
		await finish();
		return;
	}

	// The input path matches a file that's presumable a task, so load and run it.
	if (loaded_tasks.modules.length !== 1) throw Error('expected one loaded task'); // run only one task at a time
	const task = loaded_tasks.modules[0]!;
	log.info(
		`→ ${st('cyan', task.name)} ${(task.mod.task.summary && st('gray', task.mod.task.summary)) ?? ''}`,
	);

	const timing_to_run_task = timings.start('run task ' + task_name);
	const result = await run_task(
		task,
		{...args, ...to_forwarded_args(`gro ${task.name}`)},
		invoke_task,
		config,
		filer,
		log,
		timings,
	);
	timing_to_run_task();
	if (!result.ok) {
		log.info(`${st('red', '🞩')} ${st('cyan', task.name)}`);
		log_error_reasons(log, [result.reason]);
		throw result.error;
	}
	log.info(`✓ ${st('cyan', task.name)}`);

	await finish();
};
