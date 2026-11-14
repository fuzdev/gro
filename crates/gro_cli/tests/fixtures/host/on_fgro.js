// A case under `fgro run`, on the builtin module: `fgro run on_fgro.js <module> [args...]`.

import { args } from '@fuzdev/gro/args.ts';

import { run_case } from './run_case.js';

const [module, ...rest] = args;

await run_case(module, rest, (name) => import(`@fuzdev/gro/${name}.ts`));
