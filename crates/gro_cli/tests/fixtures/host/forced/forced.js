// Every builtin's first import started short of stack (`sweep.js`), in a realm nothing patched.

import { sweep } from './sweep.js';

await sweep();
