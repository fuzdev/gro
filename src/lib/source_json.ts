import {join} from 'node:path';
import {ensure_end, strip_start} from '@fuzdev/fuz_util/string.js';
import {fs_exists, fs_search} from '@fuzdev/fuz_util/fs.js';
import ts from 'typescript';
import type {PackageJson, PackageJsonExports} from '@fuzdev/fuz_util/package_json.js';
import {SourceJson, type ModuleJson, type DeclarationKind} from '@fuzdev/fuz_util/source_json.js';
import type {Logger} from '@fuzdev/fuz_util/log.js';

import {paths, replace_extension} from './paths.ts';
import {parse_exports} from './parse_exports.ts';
import {TS_MATCHER, SVELTE_MATCHER, JSON_MATCHER, CSS_MATCHER} from './constants.ts';

export type SourceJsonMapper = (
	source_json: SourceJson,
) => SourceJson | null | Promise<SourceJson | null>;

export const source_json_create = async (
	package_json: PackageJson,
	lib_path?: string,
	log?: Logger,
): Promise<SourceJson> =>
	SourceJson.parse({
		name: package_json.name,
		version: package_json.version,
		modules: await source_json_modules_create(package_json.exports, lib_path, log),
	});

export const source_json_modules_create = async (
	exports: PackageJsonExports | undefined,
	lib_path = paths.lib,
	log?: Logger,
): Promise<Array<ModuleJson> | undefined> => {
	if (!exports) return;

	const file_paths = await collect_file_paths(exports, lib_path);

	// Create a TypeScript program for all TypeScript files
	const ts_files = file_paths
		.filter(({file_path}) => TS_MATCHER.test(file_path))
		.map(({file_path}) => file_path);

	let program: ts.Program | undefined;
	if (ts_files.length > 0) {
		program = ts.createProgram(
			ts_files,
			// TODO get from tsconfig?
			{
				target: ts.ScriptTarget.ESNext,
				module: ts.ModuleKind.ESNext,
				moduleResolution: ts.ModuleResolutionKind.NodeNext,
				verbatimModuleSyntax: true,
				isolatedModules: true,
			},
		);
	}

	const result: Array<ModuleJson> = [];

	// Process each file
	for (const {file_path} of file_paths) {
		const relative_path = file_path.replace(ensure_end(lib_path, '/'), '');

		const declarations = parse_exports(file_path, program, undefined, log)
			.filter((d): d is {name: string; kind: DeclarationKind} => d.kind !== null) // TODO maybe dont filter out?
			.map(({name, kind}) => ({name, kind}));

		result.push(
			declarations.length
				? {
						path: relative_path,
						declarations,
					}
				: {
						path: relative_path,
					},
		);
	}

	return result;
};

const collect_file_paths = async (
	exports: PackageJsonExports,
	lib_path: string,
): Promise<Array<{export_key: string; file_path: string}>> => {
	const file_paths: Array<{export_key: string; file_path: string}> = [];

	// Handle string exports (single default export)
	if (typeof exports === 'string') {
		const source_file = await infer_source_from_export(exports, lib_path);
		if (source_file) {
			file_paths.push({export_key: '.', file_path: source_file});
		} else {
			throw Error(
				`Failed to infer source file from package.json string export "${exports}" - no matching file found in ${lib_path}`,
			);
		}
		return file_paths;
	}

	// Handle object exports
	for (const k in exports) {
		// Skip package.json
		if (k === './package.json') continue;

		// Check if this is a pattern export
		if (k.includes('*')) {
			// Handle pattern exports by finding matching files in lib

			const matching_files = await fs_search(lib_path, {
				file_filter: (path) => {
					const p = path.replace(ensure_end(lib_path, '/'), '');
					// Only match files in the root directory (no subdirectories)
					if (p.includes('/')) return false;

					// Match based on the export pattern
					if (k === './*.js') {
						return TS_MATCHER.test(p) && !p.endsWith('.d.ts') && !p.endsWith('.test.ts');
					} else if (k === './*.svelte') {
						return SVELTE_MATCHER.test(p);
					} else if (k === './*.json') {
						return JSON_MATCHER.test(p);
					} else if (k === './*.css') {
						return CSS_MATCHER.test(p);
					}
					return false;
				},
			});

			// Add each matching file
			for (const file of matching_files) {
				let export_path: string;
				if (file.path.endsWith('.ts')) {
					export_path = './' + file.path.replace('.ts', '.js');
				} else {
					export_path = './' + file.path;
				}
				file_paths.push({export_key: export_path, file_path: file.id});
			}
		} else {
			// Handle explicit exports (non-patterns)

			const source_file = await infer_source_from_export(k, lib_path);
			if (source_file) {
				file_paths.push({export_key: k, file_path: source_file});
			} else {
				throw Error(
					`Failed to infer source file from package.json export path ${k} - no matching file found in ${lib_path}`,
				);
			}
		}
	}

	return file_paths;
};

const infer_source_from_export = async (
	export_path: string,
	lib_path: string,
): Promise<string | null> => {
	// Handle index specially
	if (export_path === '.' || export_path === './') {
		const index_ts = join(lib_path, 'index.ts');
		if (await fs_exists(index_ts)) return index_ts;
		const index_js = join(lib_path, 'index.js');
		if (await fs_exists(index_js)) return index_js;
		return null;
	}

	const clean_path = strip_start(export_path, './');

	// For .js exports, try .ts first
	if (clean_path.endsWith('.js')) {
		const ts_path = join(lib_path, replace_extension(clean_path, '.ts'));
		if (await fs_exists(ts_path)) return ts_path;
	}

	// Try the exact path
	const exact_path = join(lib_path, clean_path);
	if (await fs_exists(exact_path)) return exact_path;

	return null;
};
