// the off-build-tree check's entry: a stripped module over the web globals, and a
// module of Gro's library, which a snapshot build loads from its own copy
import './web_globals.js';
import { TaskError } from '@fuzdev/gro/task_error.ts';

console.log(typeof TaskError);

const stripped: string = 'stripped';
console.log(stripped satisfies string);
