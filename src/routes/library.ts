import { library_json_from_modules } from '@fuzdev/fuz_util/library_json.ts';
import { modules } from 'virtual:svelte-docinfo';
import pkg_json from 'virtual:pkg.json';

export const library_json = library_json_from_modules(pkg_json, modules);
