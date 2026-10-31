import { test, expect, vi, beforeEach, describe } from 'vitest';
import { join } from 'node:path';

import { gen_inputs_match, should_trigger_gen } from '#lib/gen_helpers.ts';
import type { GenInputs } from '#lib/gen.ts';
import type { ModuleGraph } from '#lib/module_graph.ts';
import { paths } from '#lib/paths.ts';
import * as modules from '#lib/modules.ts';

import { create_mock_logger, create_mock_module_graph } from './test_helpers.ts';

vi.mock('#lib/modules.ts', async () => {
	const actual = await vi.importActual('#lib/modules.ts');
	return {
		...actual,
		load_module: vi.fn()
	};
});

const ROOT = '/project/';
const TEST_GEN_FILE = join(paths.root, 'src/example.gen.ts');
const TEST_HELPER_FILE = join(paths.root, 'src/helper.ts');
const TEST_JSON_FILE = join(paths.root, 'src/data.json');
const TEST_OTHER_FILE = join(paths.root, 'src/other.ts');

// the graph's `dependents`, reset per test to return an empty set
const dependents = vi.fn<ModuleGraph['dependents']>();

const create_graph = (): ModuleGraph => ({ ...create_mock_module_graph(), dependents });

const mock_inputs = (inputs: GenInputs | undefined): void => {
	vi.mocked(modules.load_module).mockResolvedValue({
		ok: true,
		id: TEST_GEN_FILE,
		mod: { gen: { generate: () => 'content', inputs } }
	});
};

beforeEach(() => {
	vi.clearAllMocks();
	dependents.mockResolvedValue(new Set());
});

describe('should_trigger_gen', () => {
	test('triggers on a change to the genfile itself without importing it', async () => {
		const result = await should_trigger_gen(
			TEST_GEN_FILE,
			TEST_GEN_FILE,
			create_graph(),
			create_mock_logger()
		);
		expect(result).toBe(true);
		expect(dependents).not.toHaveBeenCalled();
		expect(modules.load_module).not.toHaveBeenCalled();
	});

	test('triggers on a change to an import without importing the genfile', async () => {
		dependents.mockResolvedValue(new Set([TEST_GEN_FILE]));
		const result = await should_trigger_gen(
			TEST_GEN_FILE,
			TEST_HELPER_FILE,
			create_graph(),
			create_mock_logger()
		);
		expect(result).toBe(true);
		expect(modules.load_module).not.toHaveBeenCalled();
	});

	test("asks the graph for the changed file's dependents filtered to the genfile", async () => {
		mock_inputs(undefined);
		await should_trigger_gen(TEST_GEN_FILE, TEST_HELPER_FILE, create_graph(), create_mock_logger());
		expect(dependents).toHaveBeenCalledWith(TEST_HELPER_FILE, expect.any(Function));
		const filter = dependents.mock.calls[0]![1]!;
		expect(filter(TEST_GEN_FILE)).toBe(true);
		expect(filter(TEST_OTHER_FILE)).toBe(false);
	});

	test('triggers when the changed file matches the inputs', async () => {
		mock_inputs({ patterns: ['*.json'] });
		const graph = create_graph();
		const log = create_mock_logger();
		expect(await should_trigger_gen(TEST_GEN_FILE, TEST_JSON_FILE, graph, log)).toBe(true);
		expect(await should_trigger_gen(TEST_GEN_FILE, TEST_OTHER_FILE, graph, log)).toBe(false);
		expect(modules.load_module).toHaveBeenCalledWith(TEST_GEN_FILE, expect.any(Function));
	});

	test('triggers on every change for inputs of all', async () => {
		mock_inputs('all');
		const result = await should_trigger_gen(
			TEST_GEN_FILE,
			TEST_OTHER_FILE,
			create_graph(),
			create_mock_logger()
		);
		expect(result).toBe(true);
	});

	test('does not trigger without inputs', async () => {
		mock_inputs(undefined);
		const result = await should_trigger_gen(
			TEST_GEN_FILE,
			TEST_OTHER_FILE,
			create_graph(),
			create_mock_logger()
		);
		expect(result).toBe(false);
	});

	test('reads inputs from a function genfile as none', async () => {
		vi.mocked(modules.load_module).mockResolvedValue({
			ok: true,
			id: TEST_GEN_FILE,
			mod: { gen: () => 'content' }
		});
		const result = await should_trigger_gen(
			TEST_GEN_FILE,
			TEST_OTHER_FILE,
			create_graph(),
			create_mock_logger()
		);
		expect(result).toBe(false);
	});

	test('logs a failed import and does not trigger', async () => {
		vi.mocked(modules.load_module).mockResolvedValue({
			ok: false,
			type: 'failed_import',
			id: TEST_GEN_FILE,
			error: new Error('Module not found')
		});
		const log = create_mock_logger();
		const result = await should_trigger_gen(TEST_GEN_FILE, TEST_OTHER_FILE, create_graph(), log);
		expect(result).toBe(false);
		expect(log.error).toHaveBeenCalledWith(`Failed to import ${TEST_GEN_FILE}:`, expect.any(Error));
	});

	test('does not trigger or log for a failed validation', async () => {
		vi.mocked(modules.load_module).mockResolvedValue({
			ok: false,
			type: 'failed_validation',
			id: TEST_GEN_FILE,
			mod: {},
			validation: 'validate_gen_module'
		});
		const log = create_mock_logger();
		const result = await should_trigger_gen(TEST_GEN_FILE, TEST_OTHER_FILE, create_graph(), log);
		expect(result).toBe(false);
		expect(log.error).not.toHaveBeenCalled();
	});
});

describe('gen_inputs_match', () => {
	test('all matches every file, inside the root or not', () => {
		expect(gen_inputs_match('all', '/project/src/a.ts', ROOT)).toBe(true);
		expect(gen_inputs_match('all', '/elsewhere/a.ts', ROOT)).toBe(true);
	});

	test('nothing declared matches nothing', () => {
		expect(gen_inputs_match({}, '/project/src/a.ts', ROOT)).toBe(false);
		expect(gen_inputs_match({ files: [], patterns: [] }, '/project/src/a.ts', ROOT)).toBe(false);
	});

	test('files match exactly, relative to the root or absolute', () => {
		const inputs: GenInputs = { files: ['src/a.ts', '/project/b.json'] };
		expect(gen_inputs_match(inputs, '/project/src/a.ts', ROOT)).toBe(true);
		expect(gen_inputs_match(inputs, '/project/b.json', ROOT)).toBe(true);
		expect(gen_inputs_match(inputs, '/project/src/a.tsx', ROOT)).toBe(false);
		expect(gen_inputs_match(inputs, '/project/src/sub/a.ts', ROOT)).toBe(false);
	});

	test('files match outside the root', () => {
		const inputs: GenInputs = { files: ['../sibling/results.json'] };
		expect(gen_inputs_match(inputs, '/sibling/results.json', ROOT)).toBe(true);
	});

	test('a pattern without a slash matches at any depth', () => {
		const inputs: GenInputs = { patterns: ['*.task.ts'] };
		expect(gen_inputs_match(inputs, '/project/x.task.ts', ROOT)).toBe(true);
		expect(gen_inputs_match(inputs, '/project/src/lib/deep/x.task.ts', ROOT)).toBe(true);
		expect(gen_inputs_match(inputs, '/project/src/lib/x.ts', ROOT)).toBe(false);
	});

	test('a leading slash anchors a pattern at the root', () => {
		const inputs: GenInputs = { patterns: ['/package.json'] };
		expect(gen_inputs_match(inputs, '/project/package.json', ROOT)).toBe(true);
		expect(gen_inputs_match(inputs, '/project/src/package.json', ROOT)).toBe(false);
	});

	test('a directory pattern matches the files under it', () => {
		const inputs: GenInputs = { patterns: ['src/data/'] };
		expect(gen_inputs_match(inputs, '/project/src/data/a.json', ROOT)).toBe(true);
		expect(gen_inputs_match(inputs, '/project/src/data/sub/b.json', ROOT)).toBe(true);
		expect(gen_inputs_match(inputs, '/project/src/other/a.json', ROOT)).toBe(false);
	});

	test('a directory pattern does not match a file of that name', () => {
		expect(gen_inputs_match({ patterns: ['src/data/'] }, '/project/src/data', ROOT)).toBe(false);
	});

	test('a negated pattern excludes what an earlier one matched', () => {
		const inputs: GenInputs = { patterns: ['*.ts', '!*.test.ts'] };
		expect(gen_inputs_match(inputs, '/project/src/a.ts', ROOT)).toBe(true);
		expect(gen_inputs_match(inputs, '/project/src/a.test.ts', ROOT)).toBe(false);
	});

	test('patterns never match outside the root', () => {
		const inputs: GenInputs = { patterns: ['*.json'] };
		expect(gen_inputs_match(inputs, '/sibling/results.json', ROOT)).toBe(false);
		expect(gen_inputs_match(inputs, '/project', ROOT)).toBe(false);
		expect(gen_inputs_match({ patterns: ['*'] }, '/', ROOT)).toBe(false);
	});

	test('patterns match a root file whose name starts with dots', () => {
		const inputs: GenInputs = { patterns: ['..data.json'] };
		expect(gen_inputs_match(inputs, '/project/..data.json', ROOT)).toBe(true);
	});

	test('files and patterns combine', () => {
		const inputs: GenInputs = { files: ['src/a.ts'], patterns: ['*.json'] };
		expect(gen_inputs_match(inputs, '/project/src/a.ts', ROOT)).toBe(true);
		expect(gen_inputs_match(inputs, '/project/src/b.json', ROOT)).toBe(true);
		expect(gen_inputs_match(inputs, '/project/src/b.ts', ROOT)).toBe(false);
	});

	test('defaults the root to the project root', () => {
		expect(gen_inputs_match({ patterns: ['*.json'] }, TEST_JSON_FILE)).toBe(true);
		expect(gen_inputs_match({ files: ['src/data.json'] }, TEST_JSON_FILE)).toBe(true);
	});
});
