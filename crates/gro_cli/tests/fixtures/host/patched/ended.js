// The entry of a run that ends before it could sum anything up: in a patched realm, armed, it
// uses the realm once and then fails, as a builtin's function that ends the process would
// leave the run. The trap's report is on stdout all the same, since each is printed as it
// happens.

import { NAMES } from './names.js';
import { arm, patch_realm } from './realm.js';

await null;

patch_realm(NAMES);
arm();
Object;
throw 'ended';
