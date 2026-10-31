import { resolve } from 'node:path';
import type { Logger } from '@fuzdev/fuz_util/log.ts';
import type { Timings } from '@fuzdev/fuz_util/timings.ts';
import type { PathId } from '@fuzdev/fuz_util/path.ts';

import type { GroConfig } from './gro_config.ts';
import type { ModuleGraph } from './module_graph.ts';
import type { InvokeTask } from './task.ts';
import {
	normalize_gen_config,
	validate_gen_module,
	type GenDependencies,
	type GenDependenciesConfig,
	create_gen_context
} from './gen.ts';
import { load_module } from './modules.ts';

/**
 * Check if a file change should trigger a gen file.
 */
export const should_trigger_gen = async (
	gen_file_id: PathId,
	changed_file_id: PathId,
	config: GroConfig,
	graph: ModuleGraph,
	log: Logger,
	timings: Timings,
	invoke_task: InvokeTask
): Promise<boolean> => {
	// Always trigger if the gen file itself changed
	const is_self_change = gen_file_id === changed_file_id;
	if (is_self_change) return true;

	// Check if gen file depends on the changed file (directly or transitively)
	const dependents = await graph.dependents(
		changed_file_id,
		(id) => id === gen_file_id // filter for just our gen file
	);
	const should_bust_cache = dependents.has(gen_file_id);

	// Resolve dependencies (with cache busting if the changed file is a dependency of the gen file)
	const dependencies = await resolve_gen_dependencies(
		gen_file_id,
		changed_file_id,
		should_bust_cache,
		config,
		log,
		timings,
		invoke_task
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
	log: Logger,
	timings: Timings,
	invoke_task: InvokeTask
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
		dependencies = await dependencies(
			create_gen_context({ config, log, timings, invoke_task }, gen_file_id, changed_file_id)
		);
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
