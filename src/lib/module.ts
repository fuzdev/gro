import { LIB_DIRNAME } from './paths.ts';
import { SOURCE_DIR, SOURCE_DIRNAME } from './constants.ts';

export const MODULE_PATH_SRC_PREFIX = SOURCE_DIR;
export const MODULE_PATH_LIB_PREFIX = `$${LIB_DIRNAME}/`;

const INTERNAL_MODULE_MATCHER = new RegExp(
	`^(\\.?\\.?|${SOURCE_DIRNAME}|\\$${LIB_DIRNAME})\\/`,
	'u'
);

export const is_external_module = (module_name: string): boolean =>
	!INTERNAL_MODULE_MATCHER.test(module_name);
