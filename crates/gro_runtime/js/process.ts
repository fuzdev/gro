// The process builtin, `@fuzdev/gro/process.ts` on the runtime: the process host module's
// runtime implementation, carrying `process.args` alone so far. The Node implementation,
// `src/lib/process.ts`, is the oracle, and its `Process` interface is the one declaration of
// the type: this module declares none, and a test in `src/test` checks the namespace's inferred
// type against it.
//
// Evaluated in the snapshot, before any user code, so what it captures at its top level is the
// realm's own: the ops, and `primordials`, intrinsics copied when the runtime was made. The
// factory runs once a run, when the builtin is first imported, which can be after user code has
// patched the global intrinsics, so it reaches none of them: the op's array is made natively,
// and the records are made by `create_record`, which copied `Object`'s functions when the
// snapshot evaluated it.
//
// This file must stay 7-bit ASCII: the build fails otherwise.

import { primordials } from 'ext:core/mod.js';
import { op_gro_process_args } from 'ext:core/ops';
import { create_record } from 'ext:gro/internal/records.ts';

const { ObjectFreeze } = primordials;

/**
 * The builtin's namespace: `process`, frozen and null-prototype, with `args` the script's
 * arguments as a frozen array of strings, and the `default` every builtin has, `undefined`.
 */
export const create_process_namespace = () => {
	const process = ObjectFreeze(create_record({ args: ObjectFreeze(op_gro_process_args()) }));
	return create_record({ process, default: undefined });
};
