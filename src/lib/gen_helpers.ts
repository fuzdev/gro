import { relative, resolve } from 'node:path';
import type { Logger } from '@fuzdev/fuz_util/log.ts';
import type { PathId } from '@fuzdev/fuz_util/path.ts';
import { IgnoreStack } from '@fuzdev/tsv';

import { paths } from './paths.ts';
import type { ModuleGraph } from './module_graph.ts';
import { normalize_gen_config, validate_gen_module, type GenInputs } from './gen.ts';
import { load_module } from './modules.ts';

/**
 * Checks if a file change should rerun a genfile:
 * the genfile itself changed, it imports the changed file directly or transitively,
 * or the changed file matches its `inputs`.
 * The genfile is imported to read its `inputs` only when the graph doesn't decide.
 * Node's module cache keeps the first read of a gen (and a failed evaluation),
 * so an edit to `inputs` applies on restart.
 * @param gen_file_id - the genfile that may rerun
 * @param changed_file_id - the file that changed
 * @param graph - the module graph, for the genfile's imports
 * @param log - logs a genfile that fails to import
 */
export const should_trigger_gen = async (
	gen_file_id: PathId,
	changed_file_id: PathId,
	graph: ModuleGraph,
	log: Logger
): Promise<boolean> => {
	if (gen_file_id === changed_file_id) return true;

	const dependents = await graph.dependents(changed_file_id, (id) => id === gen_file_id);
	if (dependents.has(gen_file_id)) return true;

	const result = await load_module(gen_file_id, validate_gen_module);
	if (!result.ok) {
		if (result.type === 'failed_import') {
			log.error(`Failed to import ${gen_file_id}:`, result.error);
		}
		return false;
	}

	const { inputs } = normalize_gen_config(result.mod.gen);
	return !!inputs && gen_inputs_match(inputs, changed_file_id);
};

/**
 * Checks if a file is one of a genfile's `inputs`.
 * `'all'` matches every file, `files` match exactly after resolving against the root,
 * and `patterns` match like a root `.gitignore`, only for files inside the root.
 * @param inputs - the genfile's `inputs`
 * @param id - the file to check
 * @param root_dir - the project root that `files` resolve against and `patterns` anchor at
 */
export const gen_inputs_match = (
	inputs: GenInputs,
	id: PathId,
	root_dir: string = paths.root
): boolean => {
	if (inputs === 'all') return true;
	const { files, patterns } = inputs;
	if (files?.some((f) => resolve(root_dir, f) === id)) return true;
	if (!patterns?.length) return false;
	const path = relative(root_dir, id);
	if (path === '..' || path.startsWith('../')) return false;
	const stack = new IgnoreStack();
	stack.push_gitignore('', patterns.join('\n'));
	return stack.is_ignored(path, false);
};
