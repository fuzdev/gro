// Gro's library, loaded from the binary's own copy at its `gro:` URL, never a package
import { TaskError, is_task_error } from '@fuzdev/gro/task_error.ts';
import * as gro from '@fuzdev/gro';

console.log(import.meta.resolve('@fuzdev/gro/task_error.ts'));
console.log(is_task_error(new TaskError('task')), gro.TaskError === TaskError);
