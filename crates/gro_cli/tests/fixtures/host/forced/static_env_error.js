// A module that imports a builtin statically, loaded by `race.js` in the same turn as a deep
// `import()` of it.

import * as ns from '@fuzdev/gro/env_error.ts';

export { ns };
