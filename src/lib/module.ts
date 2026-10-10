import { SOURCE_DIR, SOURCE_DIRNAME } from './constants.ts';

export const MODULE_PATH_SRC_PREFIX = SOURCE_DIR;
export const MODULE_PATH_LIB_PREFIX = '#lib/';

// `#` specifiers are package.json subpath imports, which are always package-internal
const INTERNAL_MODULE_MATCHER = new RegExp(`^((\\.?\\.?|${SOURCE_DIRNAME})\\/|#)`, 'u');

export const is_external_module = (module_name: string): boolean =>
	!INTERNAL_MODULE_MATCHER.test(module_name);
