import { SOURCE_DIR, SOURCE_DIRNAME, SVELTEKIT_LIB_ALIAS } from './constants.ts';

export const MODULE_PATH_SRC_PREFIX = SOURCE_DIR;
export const MODULE_PATH_LIB_PREFIX = SVELTEKIT_LIB_ALIAS + '/';

const INTERNAL_MODULE_MATCHER = new RegExp(
	`^(\\.?\\.?|${SOURCE_DIRNAME}|\\${SVELTEKIT_LIB_ALIAS})\\/`,
	'u'
);

export const is_external_module = (module_name: string): boolean =>
	!INTERNAL_MODULE_MATCHER.test(module_name);
