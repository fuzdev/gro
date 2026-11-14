// The entry of the patched run: patches the realm, arms its traps, and only then imports the
// module that imports the builtins, so each builtin's implementation runs for the first time
// in a realm where every intrinsic is a trap.

import { NAMES } from './names.js';
import { arm, patch_realm } from './realm.js';

// the runtime takes this module's evaluation promise now, while `then` is the realm's own
await null;

patch_realm(NAMES);
arm();
// not awaited: the traps are armed until the imported module's own first statement
import('./use.js');
