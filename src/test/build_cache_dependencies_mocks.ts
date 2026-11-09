import { vi } from 'vitest';

import type { BuildCacheDependencies } from '#lib/build_cache_dependencies.ts';

/**
 * The dependency key components that test files mocking `build_cache_dependencies.ts` return.
 */
export const MOCK_BUILD_CACHE_DEPENDENCIES: BuildCacheDependencies = {
	gro_version: '1.0.0',
	runtime_version: 'node/v24.0.0',
	package_lock_hash: 'lock123'
};

/**
 * The `build_cache_dependencies.ts` module answering `MOCK_BUILD_CACHE_DEPENDENCIES`, for a test of a task.
 * This module imports nothing of Gro's at runtime, so a `vi.mock` factory can load it
 * while the mocked modules are still loading:
 * `vi.mock('#lib/build_cache_dependencies.ts', async () => (await import('./build_cache_dependencies_mocks.ts')).create_mock_build_cache_dependencies_module())`.
 */
export const create_mock_build_cache_dependencies_module = (): Pick<
	typeof import('#lib/build_cache_dependencies.ts'),
	'load_build_cache_dependencies'
> => ({
	load_build_cache_dependencies: vi.fn(async () => MOCK_BUILD_CACHE_DEPENDENCIES)
});
