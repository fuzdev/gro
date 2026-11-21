import type { Timings } from '@fuzdev/fuz_util/timings.ts';
import { UnreachableError } from '@fuzdev/fuz_util/error.ts';
import type { Result } from '@fuzdev/fuz_util/result.ts';
import type { PathId } from '@fuzdev/fuz_util/path.ts';

import type { ResolvedInputFile } from './input_path.ts';
import { print_path } from './paths.ts';
import { print_error_with_causes } from './internal/error_report.ts';

export interface ModuleMeta<TModule extends Record<string, any> = Record<string, any>> {
	id: PathId;
	mod: TModule;
}

export type LoadModuleResult<TModule> = Result<{ id: PathId; mod: TModule }, LoadModuleFailure>;
export type LoadModuleFailure =
	| { ok: false; type: 'failed_import'; id: PathId; error: Error }
	| {
			ok: false;
			type: 'failed_validation';
			id: PathId;
			mod: Record<string, any>;
			validation: string;
	  };

/**
 * Imports the project module at an absolute path, resolving to its namespace:
 * how Gro loads a project's files, its tasks, genfiles, and `gro.config.ts`,
 * given by the composition root. The Node Gro's is `import_module_node`,
 * `(id) => import(to_file_url(id))`.
 */
export type ImportModule = (id: PathId) => Promise<Record<string, any>>;

/**
 * Imports the module at `id` through `import_module`, validating it.
 * @param import_module - loads the module
 */
export const load_module = async <TModule extends Record<string, any>>(
	id: PathId,
	import_module: ImportModule,
	validate?: (mod: Record<string, any>) => mod is TModule
): Promise<LoadModuleResult<TModule>> => {
	let mod;
	try {
		mod = await import_module(id);
	} catch (error) {
		return { ok: false, type: 'failed_import', id, error };
	}
	if (validate && !validate(mod)) {
		return { ok: false, type: 'failed_validation', id, mod, validation: validate.name };
	}
	return { ok: true, id, mod: mod as TModule };
};

export interface LoadModulesFailure<TModuleMeta extends ModuleMeta> {
	type: 'load_module_failures';
	load_module_failures: Array<LoadModuleFailure>;
	reasons: Array<string>;
	// still return the modules and timings, deferring to the caller
	modules: Array<TModuleMeta>;
}

export type LoadModulesResult<TModuleMeta extends ModuleMeta> = Result<
	{
		modules: Array<TModuleMeta>;
	},
	LoadModulesFailure<TModuleMeta>
>;

// TODO parallelize and sort afterwards
/**
 * Imports each of the resolved input files, validating each module.
 * @param root - the absolute project root the failures name paths relative to
 * @param import_module - loads each module
 */
export const load_modules = async <
	TModule extends Record<string, any>,
	TModuleMeta extends ModuleMeta<TModule>
>(
	resolved_input_files: Array<ResolvedInputFile>,
	validate: (mod: any) => mod is TModule,
	map_module_meta: (resolved_input_file: ResolvedInputFile, mod: TModule) => TModuleMeta,
	root: PathId,
	import_module: ImportModule,
	timings?: Timings
): Promise<LoadModulesResult<TModuleMeta>> => {
	const timing_to_load_modules = timings?.start('load modules');
	const modules: Array<TModuleMeta> = [];
	const load_module_failures: Array<LoadModuleFailure> = [];
	const reasons: Array<string> = [];
	for (const resolved_input_file of resolved_input_files.values()) {
		const { id, input_path } = resolved_input_file;
		const result = await load_module(id, import_module, validate);
		if (result.ok) {
			modules.push(map_module_meta(resolved_input_file, result.mod));
		} else {
			load_module_failures.push(result);
			switch (result.type) {
				case 'failed_import': {
					reasons.push(
						`Module import ${print_path(id, root)} failed from input ${print_path(
							input_path,
							root
						)}: ${print_error_with_causes(result.error)}`
					);
					break;
				}
				case 'failed_validation': {
					reasons.push(`Module ${print_path(id, root)} failed validation '${result.validation}'.`);
					break;
				}
				default:
					throw new UnreachableError(result);
			}
		}
	}
	timing_to_load_modules?.();

	if (load_module_failures.length) {
		return {
			ok: false,
			type: 'load_module_failures',
			load_module_failures,
			reasons,
			modules
		};
	}

	return { ok: true, modules };
};
