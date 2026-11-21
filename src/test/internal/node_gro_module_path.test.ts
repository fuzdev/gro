import { test, assert, vi } from 'vitest';
import { join } from 'node:path';

import { resolve_gro_module_path } from '#lib/internal/node_gro_module_path.ts';

import { paths } from '../test_helpers.ts';

// neither a local install nor a built `dist/` in the cwd, so the running Gro's own directory
vi.mock('node:fs', async (import_original) => ({
	...(await import_original<typeof import('node:fs')>()),
	existsSync: () => false
}));

test("falls back to the running Gro's directory, the one holding its modules", () => {
	assert.strictEqual(resolve_gro_module_path('loader.ts'), join(paths.root, 'src/lib/loader.ts'));
	assert.strictEqual(resolve_gro_module_path(), join(paths.root, 'src/lib'));
});
