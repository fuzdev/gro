import {readFile, writeFile} from 'node:fs/promises';
import {join} from 'node:path';
import ts from 'typescript';
import {vi} from 'vitest';
import type {Logger} from '@fuzdev/fuz_util/log.js';
import type {Timings} from '@fuzdev/fuz_util/timings.js';
import {json_stringify_deterministic} from '@fuzdev/fuz_util/json.js';
import {hash_blake3} from '@fuzdev/fuz_util/hash_blake3.js';

import type {GroConfig} from '../lib/gro_config.ts';
import type {Filer} from '../lib/filer.ts';
import type {ParsedSvelteConfig} from '../lib/svelte_config.ts';
import type {TaskContext, InvokeTask} from '../lib/task.ts';

/**
 * Creates a mock logger for testing.
 */
export const create_mock_logger = (): Logger =>
	({
		error: vi.fn(),
		warn: vi.fn(),
		info: vi.fn(),
		debug: vi.fn(),
		raw: vi.fn(),
	}) as unknown as Logger;

/**
 * Creates a mock Gro config for testing.
 * Note: build_cache_config in overrides will be hashed during creation.
 */
export const create_mock_config = async (
	overrides: Partial<GroConfig> & {
		build_cache_config?:
			| Record<string, unknown>
			| (() => Record<string, unknown> | Promise<Record<string, unknown>>);
	} = {},
): Promise<GroConfig> => {
	// Extract and hash build_cache_config if provided
	const {build_cache_config, ...rest} = overrides;
	let build_cache_config_hash: string;

	if (!build_cache_config) {
		build_cache_config_hash = hash_blake3('');
	} else {
		// Resolve if it's a function
		const resolved =
			typeof build_cache_config === 'function' ? await build_cache_config() : build_cache_config;

		// Hash the JSON representation with deterministic key ordering
		build_cache_config_hash = hash_blake3(json_stringify_deterministic(resolved));
	}

	return {
		plugins: () => [],
		map_package_json: null,
		task_root_dirs: [],
		search_filters: [],
		js_cli: 'node',
		pm_cli: 'npm',
		build_cache_config_hash,
		...(rest as Partial<GroConfig>),
	} as GroConfig;
};

/**
 * Creates a mock fs.Stats object for a file.
 */
export const mock_file_stats = (
	size = 1024,
	options: {
		mtimeMs?: number;
		ctimeMs?: number;
		mode?: number;
	} = {},
): any => ({
	size,
	mtimeMs: options.mtimeMs ?? 1729512000000,
	ctimeMs: options.ctimeMs ?? 1729512000000,
	mode: options.mode ?? 33188,
	isDirectory: () => false,
});

/**
 * Creates a mock fs.Stats object for a directory.
 */
export const mock_dir_stats = (): any => ({
	isDirectory: () => true,
});

/**
 * Creates a mock fs.Dirent object for a file.
 */
export const mock_file_entry = (name: string): any => ({
	name,
	isDirectory: () => false,
	isFile: () => true,
});

/**
 * Creates a mock fs.Dirent object for a directory.
 */
export const mock_dir_entry = (name: string): any => ({
	name,
	isDirectory: () => true,
	isFile: () => false,
});

export const TEST_TIMEOUT_MD = 20_000;

export const SOME_PUBLIC_ENV_VAR_NAME = 'PUBLIC_SOME_PUBLIC_ENV_VAR';
export const SOME_PUBLIC_ENV_VAR_VALUE = 'SOME_PUBLIC_ENV_VAR';
const name_equals = SOME_PUBLIC_ENV_VAR_NAME + '=';
const line = name_equals + SOME_PUBLIC_ENV_VAR_VALUE;

let inited = false;

/**
 * Hacky global helper to init the test env.
 *
 * @returns boolean indicating if the env file was created or not
 */
export const init_test_env = async (
	dir = process.cwd(),
	env_filename = '.env',
): Promise<boolean> => {
	if (inited) return false;
	inited = true;

	const env_file = join(dir, env_filename);

	let contents: string;
	try {
		contents = await readFile(env_file, 'utf8');
	} catch (error) {
		if ((error as NodeJS.ErrnoException).code === 'ENOENT') {
			await writeFile(env_file, line + '\n', 'utf8');
			return true;
		}
		throw error;
	}

	const lines = contents.split('\n');
	if (lines.includes(line)) {
		return false; // already exists
	}

	let new_contents: string;
	const found_index = lines.findIndex((l) => l.startsWith(name_equals));
	if (found_index === -1) {
		// if the line does not exist, add it
		new_contents = contents + (contents.endsWith('\n') ? '' : '\n') + line + '\n';
	} else {
		// if the line exists but with a different value, replace it
		new_contents = contents.replace(new RegExp(`${SOME_PUBLIC_ENV_VAR_NAME}=.*`), line);
	}
	await writeFile(env_file, new_contents, 'utf8');

	return true;
};

/**
 * Creates a TypeScript environment for testing.
 * Change to `typescript-go` when it's more ready.
 * @see https://github.com/microsoft/typescript-go?tab=readme-ov-file#what-works-so-far
 */
export const create_ts_test_env = (
	source_code: string,
	dir: string = process.cwd(),
	virtual_files: Record<string, string> = {},
): {
	source_file: ts.SourceFile;
	checker: ts.TypeChecker;
	program: ts.Program;
	exports: Array<ts.Symbol>;
} => {
	// Create a virtual file path for testing
	const file_path = join(dir, 'virtual_test_file.ts');

	// Create a compiler host with custom module resolution
	const host = ts.createCompilerHost({});
	const original_get_source_file = host.getSourceFile.bind(host);

	// Override getSourceFile to return our test files
	host.getSourceFile = (fileName: string, languageVersion: ts.ScriptTarget) => {
		if (fileName === file_path) {
			return ts.createSourceFile(fileName, source_code, languageVersion);
		}

		// Check if we have a virtual file for this path
		for (const [virtual_path, content] of Object.entries(virtual_files)) {
			const full_path = join(dir, virtual_path);
			if (fileName === full_path) {
				return ts.createSourceFile(fileName, content, languageVersion);
			}
		}

		return original_get_source_file(fileName, languageVersion);
	};

	// TODO simplify?
	// Add custom module resolution using resolveModuleNameLiterals
	host.resolveModuleNameLiterals = (
		module_literals: ReadonlyArray<ts.StringLiteralLike>,
		containing_file: string,
		_redirected_reference: ts.ResolvedProjectReference | undefined,
		options: ts.CompilerOptions,
	): Array<ts.ResolvedModuleWithFailedLookupLocations> => {
		return module_literals.map((module_literal) => {
			const module_name = module_literal.text;

			// Handle relative imports that might be in our virtual files
			if (module_name.startsWith('./') || module_name.startsWith('../')) {
				const module_path = join(containing_file, '..', module_name);

				// Normalize the path handling for the virtual files
				for (const virtual_path of Object.keys(virtual_files)) {
					const full_path = join(dir, virtual_path);
					const normalized_module_path = module_path.replace(/\.ts$/, '') + '.ts';

					if (normalized_module_path === full_path) {
						return {
							resolvedModule: {
								resolvedFileName: full_path,
								isExternalLibraryImport: false,
								extension: ts.Extension.Ts,
							},
						};
					}
				}
			}

			// If it's our main file
			if (join(dir, module_name) === file_path) {
				return {
					resolvedModule: {
						resolvedFileName: file_path,
						isExternalLibraryImport: false,
						extension: ts.Extension.Ts,
					},
				};
			}

			// For non-virtual modules, try standard resolution
			return ts.resolveModuleName(module_name, containing_file, options, host);
		});
	};

	// Include all virtual files in the program files list
	const program_files = [file_path, ...Object.keys(virtual_files).map((path) => join(dir, path))];

	// TODO get from tsconfig?
	// Create program options
	const compiler_options: ts.CompilerOptions = {
		target: ts.ScriptTarget.ESNext,
		module: ts.ModuleKind.ESNext,
		moduleResolution: ts.ModuleResolutionKind.NodeNext,
		verbatimModuleSyntax: true,
		isolatedModules: true,
	};

	// Create a program with our virtual files
	const program = ts.createProgram(program_files, compiler_options, host);

	const source_file = program.getSourceFile(file_path)!;
	const checker = program.getTypeChecker();

	// Get the exports from the source file
	const symbol = checker.getSymbolAtLocation(source_file);
	const exports = symbol ? checker.getExportsOfModule(symbol) : [];

	return {source_file, checker, program, exports};
};

/**
 * Creates a mock Timings object for testing.
 */
export const create_mock_timings = (): Timings =>
	({
		start: vi.fn(() => vi.fn()),
	}) as unknown as Timings;

/**
 * Creates a mock Filer object for testing.
 */
export const create_mock_filer = (): Filer =>
	({
		find: vi.fn(),
		create_changeset: vi.fn(),
	}) as unknown as Filer;

/**
 * Creates a mock ParsedSvelteConfig for testing.
 */
export const create_mock_svelte_config = (): ParsedSvelteConfig =>
	({
		lib_path: 'src/lib',
		routes_path: 'src/routes',
	}) as ParsedSvelteConfig;

/**
 * Creates a mock TaskContext for testing.
 * Combines all the mock helpers into a complete task context.
 * Uses a synchronous config with a default hash for simplicity.
 *
 * @param args - Args to use in the context (can be partial if defaults are provided)
 * @param config_overrides - Partial config to override defaults
 * @param defaults - Default args to merge with provided args
 */
export const create_mock_task_context = <TArgs extends object = any>(
	args: Partial<TArgs> = {},
	config_overrides: Partial<GroConfig> = {},
	defaults?: TArgs,
): TaskContext<TArgs> => ({
	args: (defaults ? {...defaults, ...args} : args) as TArgs,
	config: {
		plugins: () => [],
		map_package_json: null,
		task_root_dirs: [],
		search_filters: [],
		js_cli: 'node',
		pm_cli: 'npm',
		build_cache_config_hash: 'test_hash',
		...config_overrides,
	} as GroConfig,
	svelte_config: create_mock_svelte_config(),
	filer: create_mock_filer(),
	log: create_mock_logger(),
	timings: create_mock_timings(),
	invoke_task: vi.fn() as unknown as InvokeTask,
});
