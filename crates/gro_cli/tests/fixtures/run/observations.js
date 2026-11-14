import { cwd } from '@fuzdev/gro/cwd.ts';
import { pid } from '@fuzdev/gro/pid.ts';
import { runtime_version } from '@fuzdev/gro/runtime_version.ts';

console.log(JSON.stringify({ cwd: cwd(), pid, runtime_version }));
