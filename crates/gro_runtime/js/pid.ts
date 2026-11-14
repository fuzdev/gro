// The pid builtin, `@fuzdev/gro/pid.ts` on the runtime: the pid host module's runtime
// implementation, the id of the running process. The Node implementation, `src/lib/pid.ts`, is
// the oracle, and its `pid` is the one declaration of the type: this module declares none, and
// a test in `src/test` checks the namespace's inferred type against it.
//
// The id is asked of the host once, when the builtin is first imported, and exported as data.
//
// Evaluated in the snapshot, before any user code (`args.ts` has the rule). The factory takes
// the scope of its importer's grant as its one argument, and a grant-free module has none, so
// its scope is `undefined`.
//
// This file must stay 7-bit ASCII: the build fails otherwise.

import { op_gro_pid } from 'ext:core/ops';
import { create_record } from 'ext:gro/internal/records.ts';

/**
 * The builtin's namespace: `pid`, the process's id, and the `default` every builtin has,
 * `undefined`.
 * @param _scope - the scope of the importer's grant, which a grant-free module has none of
 */
export const create_pid_namespace = (_scope: undefined) =>
	create_record({ pid: op_gro_pid(), default: undefined });
