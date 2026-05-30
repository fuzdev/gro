import {library_json_from_modules} from '@fuzdev/fuz_util/library_json.js';
import {modules} from 'virtual:svelte-docinfo';

import package_json from '../../package.json' with {type: 'json'};

export const library_json = library_json_from_modules(package_json, modules);
