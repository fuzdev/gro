import type {Logger} from '@fuzdev/fuz_util/log.js';
import {join, basename, dirname, isAbsolute} from 'node:path';
import {mkdir, readFile, writeFile} from 'node:fs/promises';
import type {Result} from '@fuzdev/fuz_util/result.js';
import type {Timings} from '@fuzdev/fuz_util/timings.js';
import {styleText as st} from 'node:util';
import type {PathId} from '@fuzdev/fuz_util/path.js';
import {each_concurrent, map_concurrent} from '@fuzdev/fuz_util/async.js';
import {fs_search} from '@fuzdev/fuz_util/fs.js';

import {print_path} from './paths.ts';
import type {GroConfig} from './gro_config.ts';
import type {ParsedSvelteConfig} from './svelte_config.ts';
import {load_modules, type LoadModulesFailure, type ModuleMeta} from './modules.ts';
import {
	InputPath,
	resolve_input_files,
	resolve_input_paths,
	type ResolvedInputFile,
	type ResolvedInputPath,
} from './input_path.ts';
import type {Filer} from './filer.ts';
import type {InvokeTask} from './task.ts';

export const GEN_FILE_PATTERN_TEXT = 'gen';
export const GEN_FILE_PATTERN = '.' + GEN_FILE_PATTERN_TEXT + '.';

export const is_gen_path = (path: string): boolean => path.includes(GEN_FILE_PATTERN);

export interface GenResult {
	origin_id: PathId;
	files: Array<GenFile>;
}
export interface GenFile {
	id: PathId;
	content: string;
	origin_id: PathId;
	format: boolean;
}

export type GenDependencies = 'all' | GenDependenciesConfig | GenDependenciesResolver;

export interface GenDependenciesConfig {
	patterns?: Array<RegExp>;
	files?: Array<PathId>;
}

export type GenDependenciesResolver = (
	ctx: GenContext,
) => GenDependenciesConfig | 'all' | null | Promise<GenDependenciesConfig | 'all' | null>;

export type Gen = GenFunction | GenConfig;

export type GenFunction = (ctx: GenContext) => RawGenResult | Promise<RawGenResult>;

// TODO add a GenConfigRaw variant and change `normalize_gen_config` to `gen_cook_config`
export interface GenConfig {
	generate: GenFunction;
	dependencies?: GenDependencies;
	// TODO think about what could be added
	// cache?: boolean;
}

export interface GenContext {
	config: GroConfig;
	svelte_config: ParsedSvelteConfig;
	filer: Filer;
	log: Logger;
	timings: Timings;
	invoke_task: InvokeTask;
	/**
	 * Same as `import.meta.url` but in path form.
	 */
	origin_id: PathId;
	/**
	 * The `origin_id` relative to the root dir.
	 */
	origin_path: string;
	/**
	 * The file that triggered dependency checking.
	 * Only available when resolving dependencies dynamically.
	 * `undefined` during actual generation.
	 */
	changed_file_id: PathId | undefined;
}

// TODO consider other return data - metadata? effects? non-file build artifacts?
export type RawGenResult = string | RawGenFile | null | Array<RawGenResult>;
export interface RawGenFile {
	content: string;
	// Defaults to file name without the `.gen`, and can be a relative path.
	// TODO maybe support a transform pattern or callback fn? like '[stem].thing.[ext]'
	filename?: string;
	format?: boolean; // defaults to `true`
}

export interface GenResults {
	results: Array<GenfileModuleResult>;
	successes: Array<GenfileModuleResultSuccess>;
	failures: Array<GenfileModuleResultFailure>;
	input_count: number;
	output_count: number;
	elapsed: number;
}
export type GenfileModuleResult = GenfileModuleResultSuccess | GenfileModuleResultFailure;
export interface GenfileModuleResultSuccess {
	ok: true;
	id: PathId;
	files: Array<GenFile>;
	elapsed: number;
}
export interface GenfileModuleResultFailure {
	ok: false;
	id: PathId;
	reason: string;
	error: Error;
	elapsed: number;
}

export const to_gen_result = (origin_id: PathId, raw_result: RawGenResult): GenResult => {
	return {
		origin_id,
		files: to_gen_files(origin_id, raw_result),
	};
};

const to_gen_files = (origin_id: PathId, raw_result: RawGenResult): Array<GenFile> => {
	if (raw_result === null) {
		return [];
	} else if (typeof raw_result === 'string') {
		return [to_gen_file(origin_id, {content: raw_result})];
	} else if (Array.isArray(raw_result)) {
		const files = raw_result.flatMap((f) => to_gen_files(origin_id, f));
		validate_gen_files(files);
		return files;
	}
	return [to_gen_file(origin_id, raw_result)];
};

const to_gen_file = (origin_id: PathId, raw_gen_file: RawGenFile): GenFile => {
	const {content, filename, format = true} = raw_gen_file;
	const id = to_output_file_id(origin_id, filename);
	return {id, content, origin_id, format};
};

const to_output_file_id = (origin_id: PathId, raw_file_name: string | undefined): string => {
	if (raw_file_name === '') {
		throw Error(`Output file name cannot be an empty string`);
	}
	const filename = raw_file_name ?? to_output_file_name(basename(origin_id));
	if (isAbsolute(filename)) return filename;
	const dir = dirname(origin_id);
	const output_file_id = join(dir, filename);
	if (output_file_id === origin_id) {
		throw Error('Gen origin and output file ids cannot be the same');
	}
	return output_file_id;
};

export const to_output_file_name = (filename: string): string => {
	const parts = filename.split('.');
	const gen_pattern_index = parts.indexOf(GEN_FILE_PATTERN_TEXT);
	if (gen_pattern_index === -1) {
		throw Error(`Invalid gen file name - '${GEN_FILE_PATTERN_TEXT}' not found in '${filename}'`);
	}
	if (gen_pattern_index !== parts.lastIndexOf(GEN_FILE_PATTERN_TEXT)) {
		throw Error(
			`Invalid gen file name - multiple instances of '${GEN_FILE_PATTERN_TEXT}' found in '${filename}'`,
		);
	}
	if (gen_pattern_index < parts.length - 3) {
		// This check is technically unneccessary,
		// but ensures a consistent file naming convention.
		throw Error(
			`Invalid gen file name - only one additional extension is allowed to follow '${GEN_FILE_PATTERN}' in '${filename}'`,
		);
	}
	const final_parts: Array<string> = [];
	const has_different_ext = gen_pattern_index === parts.length - 3;
	const length = has_different_ext ? parts.length - 1 : parts.length;
	for (let i = 0; i < length; i++) {
		if (i === gen_pattern_index) continue; // skip the `.gen.` pattern
		if (i === length - 1 && parts[i] === '') continue; // allow empty extension
		final_parts.push(parts[i]!);
	}
	return final_parts.join('.');
};

const validate_gen_files = (files: Array<GenFile>) => {
	const ids = new Set();
	for (const file of files) {
		if (ids.has(file.id)) {
			throw Error(`Duplicate gen file id: ${file.id}`);
		}
		ids.add(file.id);
	}
};

export type AnalyzedGenResult =
	| {
			file: GenFile;
			existing_content: string;
			is_new: false;
			has_changed: boolean;
	  }
	| {
			file: GenFile;
			existing_content: null;
			is_new: true;
			has_changed: true;
	  };

export const analyze_gen_results = async (
	gen_results: GenResults,
): Promise<Array<AnalyzedGenResult>> => {
	const files = gen_results.successes.flatMap((result) => result.files);
	return map_concurrent(files, 10, (file) => analyze_gen_result(file));
};

export const analyze_gen_result = async (file: GenFile): Promise<AnalyzedGenResult> => {
	let existing_content: string;
	try {
		existing_content = await readFile(file.id, 'utf8');
	} catch (error) {
		if (error.code === 'ENOENT') {
			return {
				file,
				existing_content: null,
				is_new: true,
				has_changed: true,
			};
		}
		throw error;
	}
	return {
		file,
		existing_content,
		is_new: false,
		has_changed: file.content !== existing_content,
	};
};

export const write_gen_results = async (
	gen_results: GenResults,
	analyzed_gen_results: Array<AnalyzedGenResult>,
	log: Logger,
): Promise<void> => {
	const files = gen_results.successes.flatMap((result) => result.files);
	await each_concurrent(files, 10, async (file) => {
		const analyzed = analyzed_gen_results.find((r) => r.file.id === file.id);
		if (!analyzed) throw Error('Expected to find analyzed result: ' + file.id);
		const log_args = [print_path(file.id), 'generated from', print_path(file.origin_id)];
		if (analyzed.is_new) {
			log.info('writing new', ...log_args);
			await mkdir(dirname(file.id), {recursive: true});
			await writeFile(file.id, file.content);
		} else if (analyzed.has_changed) {
			log.info('writing changed', ...log_args);
			await writeFile(file.id, file.content);
		} else {
			log.info('skipping unchanged', ...log_args);
		}
	});
};

export interface FoundGenfiles {
	resolved_input_files: Array<ResolvedInputFile>;
	resolved_input_files_by_root_dir: Map<PathId, Array<ResolvedInputFile>>;
	resolved_input_paths: Array<ResolvedInputPath>;
}

export type FindGenfilesResult = Result<{value: FoundGenfiles}, FindGenfilesFailure>;
export type FindGenfilesFailure =
	| {
			type: 'unmapped_input_paths';
			unmapped_input_paths: Array<InputPath>;
			resolved_input_paths: Array<ResolvedInputPath>;
			reasons: Array<string>;
	  }
	| {
			type: 'input_directories_with_no_files';
			input_directories_with_no_files: Array<InputPath>;
			resolved_input_files: Array<ResolvedInputFile>;
			resolved_input_files_by_root_dir: Map<PathId, Array<ResolvedInputFile>>;
			resolved_input_paths: Array<ResolvedInputPath>;
			reasons: Array<string>;
	  };

/**
 * Finds modules from input paths. (see `input_path.ts` for more)
 */
export const find_genfiles = async (
	input_paths: Array<InputPath>,
	root_dirs: Array<PathId>,
	config: GroConfig,
	timings?: Timings,
): Promise<FindGenfilesResult> => {
	const extensions: Array<string> = [GEN_FILE_PATTERN];

	// Check which extension variation works - if it's a directory, prefer others first!
	const timing_to_resolve_input_paths = timings?.start('resolve input paths');
	const {resolved_input_paths, unmapped_input_paths} = await resolve_input_paths(
		input_paths,
		root_dirs,
		extensions,
	);
	timing_to_resolve_input_paths?.();

	// Error if any input path could not be mapped.
	if (unmapped_input_paths.length) {
		return {
			ok: false,
			type: 'unmapped_input_paths',
			unmapped_input_paths,
			resolved_input_paths,
			reasons: unmapped_input_paths.map((input_path) =>
				st('red', `Input path ${print_path(input_path)} cannot be mapped to a file or directory.`),
			),
		};
	}

	// Find all of the files for any directories.
	const timing_to_fs_search = timings?.start('find files');
	const {resolved_input_files, resolved_input_files_by_root_dir, input_directories_with_no_files} =
		await resolve_input_files(
			resolved_input_paths,
			async (id) =>
				await fs_search(id, {
					filter: config.search_filters,
					file_filter: (p) => extensions.some((e) => p.includes(e)),
				}),
		);
	timing_to_fs_search?.();

	// Error if any input path has no files. (means we have an empty directory)
	if (input_directories_with_no_files.length) {
		return {
			ok: false,
			type: 'input_directories_with_no_files',
			input_directories_with_no_files,
			resolved_input_files,
			resolved_input_files_by_root_dir,
			resolved_input_paths,
			reasons: input_directories_with_no_files.map((input_path) =>
				st('red', `Input directory contains no matching files: ${print_path(input_path)}`),
			),
		};
	}

	return {
		ok: true,
		value: {
			resolved_input_files,
			resolved_input_files_by_root_dir,
			resolved_input_paths,
		},
	};
};

export interface GenfileModule {
	gen: Gen;
}

export type GenfileModuleMeta = ModuleMeta<GenfileModule>;

export interface LoadedGenfiles {
	modules: Array<GenfileModuleMeta>;
	found_genfiles: FoundGenfiles;
}

export type LoadGenfilesResult = Result<{value: LoadedGenfiles}, LoadGenfilesFailure>;
export type LoadGenfilesFailure = LoadModulesFailure<GenfileModuleMeta>;

export const load_genfiles = async (
	found_genfiles: FoundGenfiles,
	timings?: Timings,
): Promise<LoadGenfilesResult> => {
	const loaded_modules = await load_modules(
		found_genfiles.resolved_input_files,
		validate_gen_module,
		(resolved_input_file, mod): GenfileModuleMeta => ({id: resolved_input_file.id, mod}),
		timings,
	);
	if (!loaded_modules.ok) {
		return loaded_modules;
	}
	return {
		ok: true,
		value: {modules: loaded_modules.modules, found_genfiles},
	};
};

export const validate_gen_module = (mod: Record<string, any>): mod is GenfileModule => {
	if (typeof mod.gen === 'function') return true;
	if (typeof mod.gen === 'object' && mod.gen !== null && typeof mod.gen.generate === 'function') {
		return true;
	}
	return false;
};

export const normalize_gen_config = (gen: Gen): GenConfig =>
	typeof gen === 'function' ? {generate: gen} : gen;
