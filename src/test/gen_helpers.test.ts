import { test, expect, vi, beforeEach } from 'vitest';
import { resolve } from 'node:path';
import type { Logger } from '@fuzdev/fuz_util/log.ts';
import type { Timings } from '@fuzdev/fuz_util/timings.ts';

import { should_trigger_gen } from '#lib/gen_helpers.ts';
import type { ModuleGraph } from '#lib/module_graph.ts';
import type { GroConfig } from '#lib/gro_config.ts';
import type { InvokeTask } from '#lib/task.ts';
import * as modules from '#lib/modules.ts';

// Mock the load_module function
vi.mock('#lib/modules.ts', async () => {
	const actual = await vi.importActual('#lib/modules.ts');
	return {
		...actual,
		load_module: vi.fn()
	};
});

// Common test paths
const TEST_GEN_FILE = resolve('src/example.gen.ts');
const TEST_HELPER_FILE = resolve('src/helper.ts');
const TEST_UTILS_FILE = resolve('src/utils.ts');
const TEST_JSON_FILE = resolve('src/data.json');
const TEST_OTHER_FILE = resolve('src/other.ts');

// the graph's `dependents`, reset per test to return an empty set
const dependents = vi.fn<ModuleGraph['dependents']>();

// Helper to create a mock module graph
const create_mock_graph = (): ModuleGraph => ({
	modules: () => Promise.reject(Error('unused')),
	dependents,
	watch: () => Promise.reject(Error('unused')),
	close: () => Promise.resolve()
});

// Helper to create mock logger
const create_mock_logger = (): Logger =>
	({
		info: vi.fn(),
		error: vi.fn(),
		warn: vi.fn()
	}) as unknown as Logger;

// Helper to create mock config
const create_mock_config = (): GroConfig =>
	({
		plugins: [],
		build_cache_config_hash: 'hash123'
	}) as unknown as GroConfig;

// Helper to create mock timings
const create_mock_timings = (): Timings =>
	({
		start: vi.fn(() => vi.fn())
	}) as unknown as Timings;

// Helper to create mock invoke_task
const create_mock_invoke_task = (): InvokeTask => vi.fn();

beforeEach(() => {
	vi.clearAllMocks();
	dependents.mockResolvedValue(new Set());
});

test('should_trigger_gen returns true for self-change', async () => {
	const changed_file_id = TEST_GEN_FILE; // same file

	const result = await should_trigger_gen(
		TEST_GEN_FILE,
		changed_file_id,
		create_mock_config(),
		create_mock_graph(),
		create_mock_logger(),
		create_mock_timings(),
		create_mock_invoke_task()
	);

	expect(result).toBe(true);
	// Should not even call load_module for self-changes
	expect(modules.load_module).not.toHaveBeenCalled();
});

test('should_trigger_gen returns true for dependencies="all"', async () => {
	// Mock load_module to return dependencies: 'all'
	vi.mocked(modules.load_module).mockResolvedValue({
		ok: true,
		id: TEST_GEN_FILE,
		mod: {
			gen: {
				generate: () => 'content',
				dependencies: 'all'
			}
		}
	});

	const result = await should_trigger_gen(
		TEST_GEN_FILE,
		TEST_OTHER_FILE,
		create_mock_config(),
		create_mock_graph(),
		create_mock_logger(),
		create_mock_timings(),
		create_mock_invoke_task()
	);

	expect(result).toBe(true);
});

test('should_trigger_gen matches pattern dependencies', async () => {
	// Mock load_module to return pattern dependencies
	vi.mocked(modules.load_module).mockResolvedValue({
		ok: true,
		id: TEST_GEN_FILE,
		mod: {
			gen: {
				generate: () => 'content',
				dependencies: {
					patterns: [/\.json$/]
				}
			}
		}
	});

	// Test JSON file (should match)
	const result_json = await should_trigger_gen(
		TEST_GEN_FILE,
		TEST_JSON_FILE,
		create_mock_config(),
		create_mock_graph(),
		create_mock_logger(),
		create_mock_timings(),
		create_mock_invoke_task()
	);
	expect(result_json).toBe(true);

	// Test TS file (should not match)
	const result_ts = await should_trigger_gen(
		TEST_GEN_FILE,
		TEST_OTHER_FILE,
		create_mock_config(),
		create_mock_graph(),
		create_mock_logger(),
		create_mock_timings(),
		create_mock_invoke_task()
	);
	expect(result_ts).toBe(false);
});

test('should_trigger_gen matches specific file dependencies', async () => {
	const other_file_id = resolve('src/other.json');

	// Mock load_module to return specific file dependencies
	vi.mocked(modules.load_module).mockResolvedValue({
		ok: true,
		id: TEST_GEN_FILE,
		mod: {
			gen: {
				generate: () => 'content',
				dependencies: {
					files: [TEST_JSON_FILE]
				}
			}
		}
	});

	// Test dependency file (should match)
	const result_dep = await should_trigger_gen(
		TEST_GEN_FILE,
		TEST_JSON_FILE,
		create_mock_config(),
		create_mock_graph(),
		create_mock_logger(),
		create_mock_timings(),
		create_mock_invoke_task()
	);
	expect(result_dep).toBe(true);

	// Test other file (should not match)
	const result_other = await should_trigger_gen(
		TEST_GEN_FILE,
		other_file_id,
		create_mock_config(),
		create_mock_graph(),
		create_mock_logger(),
		create_mock_timings(),
		create_mock_invoke_task()
	);
	expect(result_other).toBe(false);
});

test('should_trigger_gen returns false when no dependencies and not self-change', async () => {
	// Mock load_module to return no dependencies
	vi.mocked(modules.load_module).mockResolvedValue({
		ok: true,
		id: TEST_GEN_FILE,
		mod: {
			gen: () => 'content' // simple function, no dependencies
		}
	});

	const result = await should_trigger_gen(
		TEST_GEN_FILE,
		TEST_OTHER_FILE,
		create_mock_config(),
		create_mock_graph(),
		create_mock_logger(),
		create_mock_timings(),
		create_mock_invoke_task()
	);

	expect(result).toBe(false);
});

test('should_trigger_gen busts cache when gen file imports changed file', async () => {
	// Mock load_module
	vi.mocked(modules.load_module).mockResolvedValue({
		ok: true,
		id: TEST_GEN_FILE,
		mod: {
			gen: {
				generate: () => 'content',
				dependencies: { files: [] }
			}
		}
	});

	// the gen file imports the helper
	dependents.mockImplementation(async (id, filter) => {
		const results: Set<string> = new Set();
		if (id === TEST_HELPER_FILE && filter) {
			// Simulate that gen file depends on helper
			if (filter(TEST_GEN_FILE)) {
				results.add(TEST_GEN_FILE);
			}
		}
		return results;
	});

	// Test: helper file change should bust cache
	await should_trigger_gen(
		TEST_GEN_FILE,
		TEST_HELPER_FILE,
		create_mock_config(),
		create_mock_graph(),
		create_mock_logger(),
		create_mock_timings(),
		create_mock_invoke_task()
	);

	// Check that load_module was called with bust_cache = true
	expect(modules.load_module).toHaveBeenCalledWith(TEST_GEN_FILE, expect.any(Function), true);

	vi.clearAllMocks();

	dependents.mockResolvedValue(new Set());

	// Test: other file change should NOT bust cache
	await should_trigger_gen(
		TEST_GEN_FILE,
		TEST_OTHER_FILE,
		create_mock_config(),
		create_mock_graph(),
		create_mock_logger(),
		create_mock_timings(),
		create_mock_invoke_task()
	);

	// Check that load_module was called with bust_cache = false
	expect(modules.load_module).toHaveBeenCalledWith(TEST_GEN_FILE, expect.any(Function), false);
});

test('should_trigger_gen passes changed_file_id to dynamic dependencies resolver', async () => {
	let captured_changed_file_id: string | undefined;

	// Mock load_module to return a dynamic dependencies function
	vi.mocked(modules.load_module).mockResolvedValue({
		ok: true,
		id: TEST_GEN_FILE,
		mod: {
			gen: {
				generate: () => 'content',
				dependencies: (ctx: any) => {
					captured_changed_file_id = ctx.changed_file_id;
					return {
						files: ctx.changed_file_id ? [ctx.changed_file_id] : []
					};
				}
			}
		}
	});

	const result = await should_trigger_gen(
		TEST_GEN_FILE,
		TEST_JSON_FILE,
		create_mock_config(),
		create_mock_graph(),
		create_mock_logger(),
		create_mock_timings(),
		create_mock_invoke_task()
	);

	expect(captured_changed_file_id).toBe(TEST_JSON_FILE);
	expect(result).toBe(true); // Should trigger because the dynamic resolver includes the changed file
});

test('should_trigger_gen handles load_module failures gracefully', async () => {
	// Mock load_module to fail
	vi.mocked(modules.load_module).mockResolvedValue({
		ok: false,
		type: 'failed_import',
		id: TEST_GEN_FILE,
		error: new Error('Module not found')
	});

	const logger = create_mock_logger();

	const result = await should_trigger_gen(
		TEST_GEN_FILE,
		TEST_OTHER_FILE,
		create_mock_config(),
		create_mock_graph(),
		logger,
		create_mock_timings(),
		create_mock_invoke_task()
	);

	expect(result).toBe(false);
	expect(logger.error).toHaveBeenCalledWith(
		`Failed to import ${TEST_GEN_FILE}:`,
		expect.any(Error)
	);
});

test('should_trigger_gen handles a changed file missing from the graph', async () => {
	const changed_file_id = resolve('src/missing.ts');

	// the graph has no dependents for a file it doesn't hold

	// Mock load_module
	vi.mocked(modules.load_module).mockResolvedValue({
		ok: true,
		id: TEST_GEN_FILE,
		mod: {
			gen: {
				generate: () => 'content',
				dependencies: { files: [] }
			}
		}
	});

	await should_trigger_gen(
		TEST_GEN_FILE,
		changed_file_id,
		create_mock_config(),
		create_mock_graph(),
		create_mock_logger(),
		create_mock_timings(),
		create_mock_invoke_task()
	);

	// Should be called with bust_cache = false since changed file doesn't exist
	expect(modules.load_module).toHaveBeenCalledWith(TEST_GEN_FILE, expect.any(Function), false);
});

test('should_trigger_gen detects transitive dependencies via the graph', async () => {
	// gen → helper → utils (transitive)
	// Mock load_module
	vi.mocked(modules.load_module).mockResolvedValue({
		ok: true,
		id: TEST_GEN_FILE,
		mod: {
			gen: {
				generate: () => 'content',
				dependencies: { files: [] }
			}
		}
	});

	// gen imports helper, helper imports utils
	dependents.mockImplementation(async (id, filter) => {
		const results: Set<string> = new Set();
		if (id === TEST_UTILS_FILE && filter) {
			// Simulate that gen file transitively depends on utils
			if (filter(TEST_GEN_FILE)) {
				results.add(TEST_GEN_FILE);
			}
		}
		return results;
	});

	await should_trigger_gen(
		TEST_GEN_FILE,
		TEST_UTILS_FILE,
		create_mock_config(),
		create_mock_graph(),
		create_mock_logger(),
		create_mock_timings(),
		create_mock_invoke_task()
	);

	// Should bust cache for transitive dependency
	expect(modules.load_module).toHaveBeenCalledWith(TEST_GEN_FILE, expect.any(Function), true);
});

test("should_trigger_gen asks the graph for the changed file's dependents", async () => {
	// Mock load_module
	vi.mocked(modules.load_module).mockResolvedValue({
		ok: true,
		id: TEST_GEN_FILE,
		mod: {
			gen: {
				generate: () => 'content',
				dependencies: { files: [] }
			}
		}
	});

	await should_trigger_gen(
		TEST_GEN_FILE,
		TEST_HELPER_FILE,
		create_mock_config(),
		create_mock_graph(),
		create_mock_logger(),
		create_mock_timings(),
		create_mock_invoke_task()
	);

	expect(dependents).toHaveBeenCalledWith(TEST_HELPER_FILE, expect.any(Function));

	// Verify the filter function only returns true for gen_file_id
	const filter_fn = dependents.mock.calls[0]![1];
	expect(filter_fn?.(TEST_GEN_FILE)).toBe(true);
	expect(filter_fn?.(resolve('src/other.gen.ts'))).toBe(false);
	expect(filter_fn?.(resolve('src/another.ts'))).toBe(false);
});

test('should_trigger_gen handles combined patterns and files dependencies', async () => {
	const specific_file_id = resolve('src/specific.ts');

	// Mock load_module to return both patterns and specific files
	vi.mocked(modules.load_module).mockResolvedValue({
		ok: true,
		id: TEST_GEN_FILE,
		mod: {
			gen: {
				generate: () => 'content',
				dependencies: {
					patterns: [/\.json$/],
					files: [specific_file_id]
				}
			}
		}
	});

	// Test JSON file (matches pattern)
	const result_json = await should_trigger_gen(
		TEST_GEN_FILE,
		TEST_JSON_FILE,
		create_mock_config(),
		create_mock_graph(),
		create_mock_logger(),
		create_mock_timings(),
		create_mock_invoke_task()
	);
	expect(result_json).toBe(true);

	// Test specific file (matches files list)
	const result_specific = await should_trigger_gen(
		TEST_GEN_FILE,
		specific_file_id,
		create_mock_config(),
		create_mock_graph(),
		create_mock_logger(),
		create_mock_timings(),
		create_mock_invoke_task()
	);
	expect(result_specific).toBe(true);

	// Test other TS file (matches neither)
	const result_other = await should_trigger_gen(
		TEST_GEN_FILE,
		TEST_OTHER_FILE,
		create_mock_config(),
		create_mock_graph(),
		create_mock_logger(),
		create_mock_timings(),
		create_mock_invoke_task()
	);
	expect(result_other).toBe(false);
});

test('should_trigger_gen handles dynamic dependency returning all', async () => {
	// Mock load_module with dynamic dependency that returns 'all'
	vi.mocked(modules.load_module).mockResolvedValue({
		ok: true,
		id: TEST_GEN_FILE,
		mod: {
			gen: {
				generate: () => 'content',
				dependencies: (_ctx: any) => 'all'
			}
		}
	});

	const result = await should_trigger_gen(
		TEST_GEN_FILE,
		TEST_OTHER_FILE,
		create_mock_config(),
		create_mock_graph(),
		create_mock_logger(),
		create_mock_timings(),
		create_mock_invoke_task()
	);

	expect(result).toBe(true);
});

test('should_trigger_gen handles validation failure from load_module', async () => {
	// Mock load_module to return validation failure
	vi.mocked(modules.load_module).mockResolvedValue({
		ok: false,
		type: 'failed_validation',
		id: TEST_GEN_FILE,
		mod: { some: 'invalid_module' },
		validation: 'validate_gen_module'
	});

	const logger = create_mock_logger();

	const result = await should_trigger_gen(
		TEST_GEN_FILE,
		TEST_OTHER_FILE,
		create_mock_config(),
		create_mock_graph(),
		logger,
		create_mock_timings(),
		create_mock_invoke_task()
	);

	expect(result).toBe(false);
	// Validation failures don't log errors (handled silently)
	expect(logger.error).not.toHaveBeenCalled();
});

test('should_trigger_gen handles async dependency resolver', async () => {
	let resolver_called = false;

	// Mock load_module with async dependency resolver
	vi.mocked(modules.load_module).mockResolvedValue({
		ok: true,
		id: TEST_GEN_FILE,
		mod: {
			gen: {
				generate: () => 'content',
				dependencies: async (ctx: any) => {
					resolver_called = true;
					// Simulate async operation
					await new Promise((resolve) => setTimeout(resolve, 0));
					return {
						files: [ctx.changed_file_id]
					};
				}
			}
		}
	});

	const result = await should_trigger_gen(
		TEST_GEN_FILE,
		TEST_JSON_FILE,
		create_mock_config(),
		create_mock_graph(),
		create_mock_logger(),
		create_mock_timings(),
		create_mock_invoke_task()
	);

	expect(resolver_called).toBe(true);
	expect(result).toBe(true);
});

test('should_trigger_gen handles null return from dependency resolver', async () => {
	// Mock load_module with dependency resolver that returns null
	vi.mocked(modules.load_module).mockResolvedValue({
		ok: true,
		id: TEST_GEN_FILE,
		mod: {
			gen: {
				generate: () => 'content',
				dependencies: (_ctx: any) => null
			}
		}
	});

	const result = await should_trigger_gen(
		TEST_GEN_FILE,
		TEST_JSON_FILE,
		create_mock_config(),
		create_mock_graph(),
		create_mock_logger(),
		create_mock_timings(),
		create_mock_invoke_task()
	);

	// null dependencies means no additional dependencies, so should not trigger
	expect(result).toBe(false);
});

test('should_trigger_gen treats empty object and null as equivalent', async () => {
	// Test with empty object
	vi.mocked(modules.load_module).mockResolvedValue({
		ok: true,
		id: TEST_GEN_FILE,
		mod: {
			gen: {
				generate: () => 'content',
				dependencies: (_ctx: any) => ({})
			}
		}
	});

	const result_empty = await should_trigger_gen(
		TEST_GEN_FILE,
		TEST_JSON_FILE,
		create_mock_config(),
		create_mock_graph(),
		create_mock_logger(),
		create_mock_timings(),
		create_mock_invoke_task()
	);

	// Test with null
	vi.mocked(modules.load_module).mockResolvedValue({
		ok: true,
		id: TEST_GEN_FILE,
		mod: {
			gen: {
				generate: () => 'content',
				dependencies: (_ctx: any) => null
			}
		}
	});

	const result_null = await should_trigger_gen(
		TEST_GEN_FILE,
		TEST_JSON_FILE,
		create_mock_config(),
		create_mock_graph(),
		create_mock_logger(),
		create_mock_timings(),
		create_mock_invoke_task()
	);

	// Both should have the same behavior
	expect(result_empty).toBe(false);
	expect(result_null).toBe(false);
});

test('should_trigger_gen handles async null return from dependency resolver', async () => {
	// Mock load_module with async dependency resolver that returns null
	vi.mocked(modules.load_module).mockResolvedValue({
		ok: true,
		id: TEST_GEN_FILE,
		mod: {
			gen: {
				generate: () => 'content',
				dependencies: async (_ctx: any) => {
					await new Promise((resolve) => setTimeout(resolve, 0));
					return null;
				}
			}
		}
	});

	const result = await should_trigger_gen(
		TEST_GEN_FILE,
		TEST_JSON_FILE,
		create_mock_config(),
		create_mock_graph(),
		create_mock_logger(),
		create_mock_timings(),
		create_mock_invoke_task()
	);

	expect(result).toBe(false);
});
