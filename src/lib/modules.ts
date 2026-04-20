import type {Timings} from '@fuzdev/fuz_util/timings.js';
import {UnreachableError} from '@fuzdev/fuz_util/error.js';
import type {Result} from '@fuzdev/fuz_util/result.js';
import {print_error} from '@fuzdev/fuz_util/print.js';
import {pathToFileURL} from 'node:url';
import type {PathId} from '@fuzdev/fuz_util/path.js';

import type {ResolvedInputFile} from './input_path.ts';
import {print_path} from './paths.ts';

export interface ModuleMeta<TModule extends Record<string, any> = Record<string, any>> {
	id: PathId;
	mod: TModule;
}

export type LoadModuleResult<TModule> = Result<{id: PathId; mod: TModule}, LoadModuleFailure>;
export type LoadModuleFailure =
	| {ok: false; type: 'failed_import'; id: PathId; error: Error}
	| {
			ok: false;
			type: 'failed_validation';
			id: PathId;
			mod: Record<string, any>;
			validation: string;
	  };

export const load_module = async <TModule extends Record<string, any>>(
	id: PathId,
	validate?: (mod: Record<string, any>) => mod is TModule,
	bust_cache?: boolean,
): Promise<LoadModuleResult<TModule>> => {
	let mod;
	try {
		let import_path = id;
		if (bust_cache) {
			const url = pathToFileURL(id);
			url.searchParams.set('t', Date.now().toString());
			import_path = url.href;
		}
		mod = await import(import_path);
	} catch (error) {
		return {ok: false, type: 'failed_import', id, error};
	}
	if (validate && !validate(mod)) {
		return {ok: false, type: 'failed_validation', id, mod, validation: validate.name};
	}
	return {ok: true, id, mod};
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
export const load_modules = async <
	TModule extends Record<string, any>,
	TModuleMeta extends ModuleMeta<TModule>,
>(
	resolved_input_files: Array<ResolvedInputFile>,
	validate: (mod: any) => mod is TModule,
	map_module_meta: (resolved_input_file: ResolvedInputFile, mod: TModule) => TModuleMeta,
	timings?: Timings,
): Promise<LoadModulesResult<TModuleMeta>> => {
	const timing_to_load_modules = timings?.start('load modules');
	const modules: Array<TModuleMeta> = [];
	const load_module_failures: Array<LoadModuleFailure> = [];
	const reasons: Array<string> = [];
	for (const resolved_input_file of resolved_input_files.values()) {
		const {id, input_path} = resolved_input_file;
		const result = await load_module(id, validate);
		if (result.ok) {
			modules.push(map_module_meta(resolved_input_file, result.mod));
		} else {
			load_module_failures.push(result);
			switch (result.type) {
				case 'failed_import': {
					reasons.push(
						`Module import ${print_path(id)} failed from input ${print_path(
							input_path,
						)}: ${print_error(result.error)}`,
					);
					break;
				}
				case 'failed_validation': {
					reasons.push(`Module ${print_path(id)} failed validation '${result.validation}'.`);
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
			modules,
		};
	}

	return {ok: true, modules};
};
