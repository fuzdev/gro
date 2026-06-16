import {resolve} from 'node:path';
import type {Logger} from '@fuzdev/fuz_util/log.ts';
import type {Timings} from '@fuzdev/fuz_util/timings.ts';
import type {PathId} from '@fuzdev/fuz_util/path.ts';

import type {GroConfig} from './gro_config.ts';
import {filter_dependents, type Filer} from './filer.ts';
import type {InvokeTask} from './task.ts';
import {
	normalize_gen_config,
	validate_gen_module,
	type GenContext,
	type GenDependencies,
	type GenDependenciesConfig,
} from './gen.ts';
import {default_svelte_config} from './svelte_config.ts';
import {to_root_path} from './paths.ts';
import {load_module} from './modules.ts';

/**
 * Check if a file change should trigger a gen file.
 */
export const should_trigger_gen = async (
	gen_file_id: PathId,
	changed_file_id: PathId,
	config: GroConfig,
	filer: Filer,
	log: Logger,
	timings: Timings,
	invoke_task: InvokeTask,
): Promise<boolean> => {
	// Always trigger if the gen file itself changed
	const is_self_change = gen_file_id === changed_file_id;
	if (is_self_change) return true;

	// Check if gen file depends on the changed file (directly or transitively)
	const changed_disknode = filer.get_by_id(changed_file_id);
	let should_bust_cache = false;
	if (changed_disknode) {
		const dependents = filter_dependents(
			changed_disknode,
			filer.get_by_id,
			(id) => id === gen_file_id, // filter for just our gen file
		);
		should_bust_cache = dependents.has(gen_file_id);
	}

	// Resolve dependencies (with cache busting if the changed file is a dependency of the gen file)
	const dependencies = await resolve_gen_dependencies(
		gen_file_id,
		changed_file_id,
		should_bust_cache,
		config,
		filer,
		log,
		timings,
		invoke_task,
	);

	if (!dependencies) return false;

	return (
		dependencies === 'all' ||
		dependencies.patterns?.some((p) => p.test(changed_file_id)) ||
		dependencies.files?.includes(changed_file_id) ||
		false
	);
};

/**
 * Resolve dependencies for a gen file.
 * Uses cache-busting only when the gen file itself changes,
 * otherwise relies on Node's module caching.
 */
const resolve_gen_dependencies = async (
	gen_file_id: string,
	changed_file_id: PathId | undefined,
	bust_cache: boolean,
	config: GroConfig,
	filer: Filer,
	log: Logger,
	timings: Timings,
	invoke_task: InvokeTask,
): Promise<GenDependenciesConfig | 'all' | null> => {
	const result = await load_module(gen_file_id, validate_gen_module, bust_cache);

	if (!result.ok) {
		if (result.type === 'failed_import') {
			log.error(`Failed to import ${gen_file_id}:`, result.error);
		}
		return null;
	}

	const gen_config = normalize_gen_config(result.mod.gen);
	if (!gen_config.dependencies) {
		return null;
	}

	let dependencies: GenDependencies | null = gen_config.dependencies;
	if (typeof dependencies === 'function') {
		const gen_ctx: GenContext = {
			config,
			svelte_config: default_svelte_config,
			filer,
			log,
			timings,
			invoke_task,
			origin_id: gen_file_id,
			origin_path: to_root_path(gen_file_id),
			changed_file_id,
		};
		dependencies = await dependencies(gen_ctx);
	}

	if (dependencies === null || dependencies === 'all') {
		return dependencies;
	}

	// At this point, dependencies is either 'all' or GenDependenciesConfig
	// For static dependencies, also normalize empty objects
	if (!dependencies.patterns?.length && !dependencies.files?.length) {
		return null;
	}

	// Normalize file paths to absolute paths
	if (dependencies.files) {
		dependencies.files = dependencies.files.map((f) => resolve(f));
	}

	return dependencies;
};
