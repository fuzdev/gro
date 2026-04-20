import type {Args} from '@fuzdev/fuz_util/args.js';
import {UnreachableError} from '@fuzdev/fuz_util/error.js';
import {EMPTY_OBJECT} from '@fuzdev/fuz_util/object.js';
import {throttle} from '@fuzdev/fuz_util/throttle.js';

import type {Plugin} from './plugin.ts';
import {paths} from './paths.ts';
import {find_genfiles, is_gen_path} from './gen.ts';
import {filter_dependents} from './filer.ts';
import {should_trigger_gen} from './gen_helpers.ts';
import {spawn_cli} from './cli.ts';

const FLUSH_DEBOUNCE_DELAY = 500;

// TODO is cache busting a good idea here to speed up and run in-process?
// 	await invoke_task('gen', {_: files, bust_cache: true});
const gen = (files: Array<string> = []) => spawn_cli('gro', ['gen', ...files]);

export interface TaskArgs extends Args {
	watch?: boolean;
}

export interface GroPluginGenOptions {
	input_paths?: Array<string>;
	root_dirs?: Array<string>;
	flush_debounce_delay?: number;
}

export const gro_plugin_gen = ({
	input_paths = [paths.source],
	root_dirs = [paths.source],
	flush_debounce_delay = FLUSH_DEBOUNCE_DELAY,
}: GroPluginGenOptions = EMPTY_OBJECT): Plugin => {
	const queued_files: Set<string> = new Set();

	let cleanup_watch: (() => void) | undefined;

	return {
		name: 'gro_plugin_gen',
		setup: async ({watch, dev, log, config, filer, invoke_task, timings}) => {
			// For production builds, we assume `gen` is already fresh,
			// which should be checked by CI via `gro check` which calls `gro gen --check`.
			if (!dev) return;

			// Do we need to just generate everything once and exit?
			if (!watch) {
				// Run `gen`, first checking if there are any modules to avoid a console error.
				// Some parts of the build may have already happened,
				// making us miss `build` events for gen dependencies,
				// so we run a full `gen` here even if it's usually wasteful.
				const found = await find_genfiles(input_paths, root_dirs, config);
				if (found.ok && found.value.resolved_input_files.length > 0) {
					await gen();
				}
				return;
			}

			const queue_gen = (gen_file_id: string) => {
				queued_files.add(gen_file_id);
				void flush_gen_queue();
			};

			const flush_gen_queue = throttle(
				async () => {
					const files = Array.from(queued_files);
					log.info(
						files.length === 0
							? '[gen] generating all files'
							: `[gen] generating ${files.length} file${files.length === 1 ? '' : 's'}`,
					);
					queued_files.clear();
					await gen(files);

					// run again?
					if (queued_files.size > 0) {
						log.info(
							`[gen] re-running for ${queued_files.size} more queued file${queued_files.size === 1 ? '' : 's'}`,
						);
						setTimeout(flush_gen_queue); // setTimeout is needed bc of throttle behavior
					} else {
						log.info('[gen] queue empty, done');
					}
				},
				{delay: flush_debounce_delay, when: 'trailing'},
			);

			// When a file builds, check it and its tree of dependents
			// for any `.gen.` files that need to run.
			cleanup_watch = await filer.watch(async (change, source_file) => {
				if (source_file.external) return;
				switch (change.type) {
					case 'add':
					case 'update': {
						// Queue the gen file itself if it changed
						if (is_gen_path(source_file.id)) {
							queue_gen(source_file.id);
						}

						// Find all current gen files and check their dependencies
						const gen_files = filer.filter((d) => !d.external && is_gen_path(d.id));
						if (gen_files) {
							for (const gen_file of gen_files) {
								const should_trigger = await should_trigger_gen(
									gen_file.id,
									source_file.id,
									config,
									filer,
									log,
									timings,
									invoke_task,
								);
								if (should_trigger) {
									queue_gen(gen_file.id);
								}
							}
						}

						// Check import-based dependents
						const dependent_gen_file_ids = filter_dependents(
							source_file,
							filer.get_by_id,
							is_gen_path,
							undefined,
							undefined,
							log,
						);
						for (const dependent_gen_file_id of dependent_gen_file_ids) {
							queue_gen(dependent_gen_file_id);
						}
						break;
					}
					case 'delete': {
						// I think for the gen plugin this is best as a no-op? avoids broken attempts
						break;
					}
					default:
						throw new UnreachableError(change.type);
				}
			});
		},
		teardown: () => {
			if (cleanup_watch) {
				cleanup_watch();
				cleanup_watch = undefined;
			}
		},
	};
};
