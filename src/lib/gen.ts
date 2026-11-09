import type { Logger } from '@fuzdev/fuz_util/log.ts';
import { join, basename, dirname, isAbsolute } from 'node:path';
import type { Result } from '@fuzdev/fuz_util/result.ts';
import type { Timings } from '@fuzdev/fuz_util/timings.ts';
import { styleText as st } from 'node:util';
import type { PathId } from '@fuzdev/fuz_util/path.ts';
import { each_concurrent, map_concurrent } from '@fuzdev/fuz_util/async.ts';
import { ensure_end } from '@fuzdev/fuz_util/string.ts';

import { paths, print_path, to_root_path, to_search_filters_relative } from './paths.ts';
import type { GroConfig } from './gro_config.ts';
import { load_default_svelte_config, type ParsedSvelteConfig } from './svelte_config.ts';
import { load_modules, type LoadModulesFailure, type ModuleMeta } from './modules.ts';
import {
	InputPath,
	resolve_input_files,
	resolve_input_paths,
	type ResolvedInputFile,
	type ResolvedInputPath
} from './input_path.ts';
import { TaskError, type InvokeTask } from './task.ts';
import type { FsOpenedPath, FsPaths } from './fs_paths.ts';

export const GEN_FILE_PATTERN_TEXT = 'gen';
export const GEN_FILE_PATTERN = '.' + GEN_FILE_PATTERN_TEXT + '.';

/**
 * Tests whether a path names a genfile by its file name,
 * so a directory like `my.gen.x/` doesn't make every file under it a genfile.
 */
export const is_gen_path = (path: string): boolean => basename(path).includes(GEN_FILE_PATTERN);

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

/**
 * What a genfile reads besides its imports, so `gro dev` reruns it when one changes:
 * `'all'` for every change the dev watch reports,
 * or files and patterns that add to its imports.
 * Plain data, read from the genfile's `gen` export.
 * `gro dev` sees changes only under `GroConfig.watch.dir` (default `src/`),
 * so an input outside it is declared but doesn't trigger a rerun.
 */
export type GenInputs =
	| 'all'
	| {
			/**
			 * Exact paths, relative to the project root or absolute, possibly outside the root.
			 */
			files?: Array<string>;
			/**
			 * Gitignore-syntax patterns anchored at the project root, like a root `.gitignore`,
			 * so `*.json` matches at any depth, `/package.json` only at the root,
			 * and `!` excludes a file an earlier pattern matched.
			 * They match only files inside the root,
			 * so one starting `./` or `../` or holding the root's absolute path is refused.
			 */
			patterns?: Array<string>;
	  };

export type Gen = GenFunction | GenConfig;

export type GenFunction = (ctx: GenContext) => RawGenResult | Promise<RawGenResult>;

// TODO add a GenConfigRaw variant and change `normalize_gen_config` to `gen_cook_config`
export interface GenConfig {
	generate: GenFunction;
	/**
	 * What the genfile reads besides its imports.
	 * Changes to its imports and to the genfile itself always rerun it.
	 */
	inputs?: GenInputs;
	// TODO think about what could be added
	// cache?: boolean;
}

export interface GenContext {
	config: GroConfig;
	/**
	 * Resolved on first access, so genfiles that don't touch it
	 * never pay to read the SvelteKit config.
	 */
	svelte_config: Promise<ParsedSvelteConfig>;
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
}

/**
 * The `GenContext` values that are the same for every genfile in a run,
 * as opposed to the per-genfile ones `create_gen_context` derives.
 */
export type GenContextBase = Pick<GenContext, 'config' | 'log' | 'timings' | 'invoke_task'>;

/**
 * Builds a `GenContext` for one genfile, with a lazy `svelte_config`
 * that's a getter rather than a value so a genfile that ignores it never resolves it.
 */
export const create_gen_context = (base: GenContextBase, origin_id: PathId): GenContext => ({
	...base,
	get svelte_config() {
		return load_default_svelte_config();
	},
	origin_id,
	origin_path: to_root_path(origin_id)
});

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

/**
 * Normalizes what a genfile returned into its output files.
 * @throws `TaskError` - for a result that's not a `RawGenResult`, like a file whose `content`
 * isn't a string, naming the genfile and what it returned
 * @throws `Error` - for an invalid or duplicate output file name
 */
export const to_gen_result = (origin_id: PathId, raw_result: RawGenResult): GenResult => {
	return {
		origin_id,
		files: to_gen_files(origin_id, raw_result)
	};
};

const to_gen_files = (origin_id: PathId, raw_result: RawGenResult): Array<GenFile> => {
	if (raw_result === null) {
		return [];
	} else if (typeof raw_result === 'string') {
		return [to_gen_file(origin_id, { content: raw_result })];
	} else if (Array.isArray(raw_result)) {
		const files = raw_result.flatMap((f) => to_gen_files(origin_id, f));
		validate_gen_files(files);
		return files;
	}
	if (typeof raw_result !== 'object' || typeof raw_result.content !== 'string') {
		throw new TaskError(
			`Genfile ${print_path(origin_id)} returned ${describe_raw_gen_result(raw_result)},` +
				' expected a string, `{content: string}`, an array of them, or `null`' +
				' (a `gen` export that is a factory, like `gen_task_registry`, must be called)'
		);
	}
	return [to_gen_file(origin_id, raw_result)];
};

// what a genfile returned, for the error when it's no `RawGenResult`
const describe_raw_gen_result = (raw_result: unknown): string => {
	if (raw_result === undefined) return 'undefined';
	if (typeof raw_result !== 'object' || raw_result === null) return 'a ' + typeof raw_result;
	const keys = Object.keys(raw_result);
	const content = (raw_result as { content?: unknown }).content;
	return (
		`an object with ${keys.length ? 'keys ' + keys.map((k) => `\`${k}\``).join(', ') : 'no keys'}` +
		` whose \`content\` is ${content === undefined ? 'missing' : content === null ? 'null' : 'a ' + typeof content}`
	);
};

const to_gen_file = (origin_id: PathId, raw_gen_file: RawGenFile): GenFile => {
	const { content, filename, format = true } = raw_gen_file;
	const id = to_output_file_id(origin_id, filename);
	return { id, content, origin_id, format };
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
			`Invalid gen file name - multiple instances of '${GEN_FILE_PATTERN_TEXT}' found in '${
				filename
			}'`
		);
	}
	if (gen_pattern_index < parts.length - 3) {
		// This check is technically unneccessary,
		// but ensures a consistent file naming convention.
		throw Error(
			`Invalid gen file name - only one additional extension is allowed to follow '${
				GEN_FILE_PATTERN
			}' in '${filename}'`
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

/**
 * Reads each output of the successful genfiles to say whether it's new or changed.
 * @param fs_paths - opens the outputs, which must resolve beneath its project
 */
export const analyze_gen_results = async (
	fs_paths: FsPaths,
	gen_results: GenResults
): Promise<Array<AnalyzedGenResult>> => {
	const files = gen_results.successes.flatMap((result) => result.files);
	return map_concurrent(files, 10, (file) => analyze_gen_result(fs_paths, file));
};

/**
 * Opens a gen output's path, its symlinks followed, refusing one that resolves outside the
 * project or to the project directory itself, so a genfile writes and `gro gen --check`
 * reads only the project's files.
 * A relative `filename` that climbs out of the genfile's directory but stays in the project,
 * like `'../elsewhere/x.ts'`, is fine.
 * @throws `TaskError` - for an output outside the project or at the project directory itself
 */
const open_gen_output = async (fs_paths: FsPaths, file: GenFile): Promise<FsOpenedPath> => {
	const output = await fs_paths.open_path(file.id);
	if (output.dir !== fs_paths.project) {
		throw new TaskError(
			`Gen output ${file.id} from ${print_path(file.origin_id)} resolves outside the project` +
				` (${output.path ? output.dir.path + '/' + output.path : output.dir.path}),` +
				' and genfiles write only inside it'
		);
	}
	if (!output.path) {
		throw new TaskError(
			`Gen output ${file.id} from ${print_path(file.origin_id)} resolves to the project` +
				' directory itself, which a genfile can never write'
		);
	}
	return output;
};

/**
 * Reads a gen output to say whether it's new or changed.
 * @param fs_paths - opens the output, which must resolve beneath its project
 * @throws `TaskError` - for an output outside the project or at the project directory itself
 */
export const analyze_gen_result = async (
	fs_paths: FsPaths,
	file: GenFile
): Promise<AnalyzedGenResult> => {
	const output = await open_gen_output(fs_paths, file);
	const existing_content = await output.dir.read_text_if_exists(output.path);
	if (existing_content === null) {
		return {
			file,
			existing_content: null,
			is_new: true,
			has_changed: true
		};
	}
	return {
		file,
		existing_content,
		is_new: false,
		has_changed: file.content !== existing_content
	};
};

/**
 * Writes the new and changed outputs of the successful genfiles.
 * @param fs_paths - opens the outputs, which must resolve beneath its project
 * @throws `TaskError` - for an output outside the project or at the project directory itself
 */
export const write_gen_results = async (
	fs_paths: FsPaths,
	gen_results: GenResults,
	analyzed_gen_results: Array<AnalyzedGenResult>,
	log: Logger
): Promise<void> => {
	const files = gen_results.successes.flatMap((result) => result.files);
	await each_concurrent(files, 10, async (file) => {
		const analyzed = analyzed_gen_results.find((r) => r.file.id === file.id);
		if (!analyzed) throw Error('Expected to find analyzed result: ' + file.id);
		const log_args = [print_path(file.id), 'generated from', print_path(file.origin_id)];
		if (analyzed.is_new) {
			log.info('writing new', ...log_args);
			const output = await open_gen_output(fs_paths, file);
			const parent = dirname(output.path);
			if (parent !== '.') await output.dir.mkdir(parent, { recursive: true });
			await write_gen_output(fs_paths, file);
		} else if (analyzed.has_changed) {
			log.info('writing changed', ...log_args);
			await write_gen_output(fs_paths, file);
		} else {
			log.info('skipping unchanged', ...log_args);
		}
	});
};

// opened at the write, after any directory it needs is made
const write_gen_output = async (fs_paths: FsPaths, file: GenFile): Promise<void> => {
	const output = await open_gen_output(fs_paths, file);
	await output.dir.write(output.path, file.content);
};

export interface FoundGenfiles {
	resolved_input_files: Array<ResolvedInputFile>;
	resolved_input_paths: Array<ResolvedInputPath>;
}

export type FindGenfilesResult = Result<{ value: FoundGenfiles }, FindGenfilesFailure>;
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
			resolved_input_paths: Array<ResolvedInputPath>;
			reasons: Array<string>;
	  };

/**
 * Finds modules from input paths. (see `input_path.ts` for more)
 * @param fs_paths - stats the input paths and searches the directories among them
 * @param root - the project root `config.search_filters` test paths relative to,
 * defaults to the project of `fs_paths`
 */
export const find_genfiles = async (
	fs_paths: FsPaths,
	input_paths: Array<InputPath>,
	root_dirs: Array<PathId>,
	config: GroConfig,
	timings?: Timings,
	root: PathId = fs_paths.project.path
): Promise<FindGenfilesResult> => {
	const extensions: Array<string> = [GEN_FILE_PATTERN];

	// Check which extension variation works - if it's a directory, prefer others first!
	const timing_to_resolve_input_paths = timings?.start('resolve input paths');
	const { resolved_input_paths, unmapped_input_paths } = await resolve_input_paths(
		fs_paths,
		input_paths,
		root_dirs,
		extensions
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
				st('red', `Input path ${print_path(input_path)} cannot be mapped to a file or directory.`)
			)
		};
	}

	// Find all of the files for any directories.
	const timing_to_fs_search = timings?.start('find files');
	const { resolved_input_files, input_directories_with_no_files } = await resolve_input_files(
		resolved_input_paths,
		async (id) =>
			await fs_paths.search_path(id, {
				filter: to_search_filters_relative(config.search_filters, root, id),
				file_filter: is_gen_path
			})
	);
	timing_to_fs_search?.();

	// Error if any input path has no files. (means we have an empty directory)
	if (input_directories_with_no_files.length) {
		return {
			ok: false,
			type: 'input_directories_with_no_files',
			input_directories_with_no_files,
			resolved_input_files,
			resolved_input_paths,
			reasons: input_directories_with_no_files.map((input_path) =>
				st('red', `Input directory contains no matching files: ${print_path(input_path)}`)
			)
		};
	}

	return {
		ok: true,
		value: {
			resolved_input_files,
			resolved_input_paths
		}
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

export type LoadGenfilesResult = Result<{ value: LoadedGenfiles }, LoadGenfilesFailure>;
export type LoadGenfilesFailure = LoadModulesFailure<GenfileModuleMeta>;

export const load_genfiles = async (
	found_genfiles: FoundGenfiles,
	timings?: Timings
): Promise<LoadGenfilesResult> => {
	const loaded_modules = await load_modules(
		found_genfiles.resolved_input_files,
		validate_gen_module,
		(resolved_input_file, mod): GenfileModuleMeta => ({ id: resolved_input_file.id, mod }),
		timings
	);
	if (!loaded_modules.ok) {
		return loaded_modules;
	}
	return {
		ok: true,
		value: { modules: loaded_modules.modules, found_genfiles }
	};
};

export const validate_gen_module = (mod: Record<string, any>): mod is GenfileModule => {
	if (typeof mod.gen === 'function') return true;
	if (typeof mod.gen === 'object' && mod.gen !== null && typeof mod.gen.generate === 'function') {
		return true;
	}
	return false;
};

/**
 * Normalizes a genfile's `gen` export to a `GenConfig`, validating its `inputs`.
 * @param gen - the genfile's `gen` export
 * @param id - the genfile, named in the error
 * @param root - the project root that `inputs` patterns anchor at
 * @throws `TaskError` - for `inputs` that can never match as written, saying what to write instead
 */
export const normalize_gen_config = (
	gen: Gen,
	id: PathId,
	root: PathId = paths.root
): GenConfig => {
	if (typeof gen === 'function') return { generate: gen };
	const problem = to_gen_inputs_problem(gen.inputs, root);
	if (problem !== null) {
		throw new TaskError(`Genfile ${print_path(id)} has invalid \`inputs\`: ${problem}`);
	}
	return gen;
};

const is_string_array = (value: unknown): value is Array<string> =>
	Array.isArray(value) && value.every((v) => typeof v === 'string');

// `inputs` come from a genfile's export at runtime, so the types don't guarantee them
const to_gen_inputs_problem = (inputs: unknown, root: PathId): string | null => {
	if (inputs === undefined || inputs === 'all') return null;
	if (typeof inputs !== 'object' || inputs === null || Array.isArray(inputs)) {
		return "expected `'all'` or an object with `files` and `patterns`";
	}
	const { files, patterns } = inputs as { files?: unknown; patterns?: unknown };
	if (files !== undefined && !is_string_array(files)) return '`files` must be an array of strings';
	if (patterns === undefined) return null;
	if (!is_string_array(patterns)) return '`patterns` must be an array of strings';
	for (const pattern of patterns) {
		const problem = to_gen_pattern_problem(pattern, root);
		if (problem !== null) return problem;
	}
	return null;
};

// a leading `/` is gitignore's root anchor, so only a path through the root itself is absolute
const to_gen_pattern_problem = (pattern: string, root: PathId): string | null => {
	const negation = pattern.startsWith('!') ? '!' : '';
	const path = pattern.slice(negation.length);
	const root_prefix = ensure_end(root, '/');
	const replacement = path.startsWith('./')
		? '/' + path.slice(2)
		: root_prefix !== '/' && path.startsWith(root_prefix)
			? '/' + path.slice(root_prefix.length)
			: null;
	if (replacement !== null) {
		return (
			`pattern \`${pattern}\` can never match, as patterns are relative to the project root` +
			` like a root \`.gitignore\`, so write \`${negation}${replacement}\``
		);
	}
	if (path.startsWith('../')) {
		return (
			`pattern \`${pattern}\` can never match, as patterns match only files inside the project` +
			' root, so name a file outside it in `files`'
		);
	}
	return null;
};
