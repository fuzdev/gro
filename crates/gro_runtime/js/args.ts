// The args builtin, `@fuzdev/gro/args.ts` on the runtime: the args host module's runtime
// implementation, the script's arguments. The Node implementation, `src/lib/args.ts`, is the
// oracle, and its `args` is the one declaration of the type: this module declares none, and a
// test in `src/test` checks the namespace's inferred type against it.
//
// Evaluated in the snapshot, before any user code, so what it captures at its top level is the
// realm's own: the op, and `primordials`, intrinsics copied when the runtime was made. The
// factory runs once a run, when the builtin is first imported, which can be after user code has
// patched the global intrinsics, so it reaches none of them: the op's array is made natively,
// and the namespace is made by `create_record`, which copied `Object`'s functions when the
// snapshot evaluated it.
//
// This file must stay 7-bit ASCII: the build fails otherwise.

import { primordials } from 'ext:core/mod.js';
import { op_gro_args } from 'ext:core/ops';
import { create_record } from 'ext:gro/internal/records.ts';

const { ObjectFreeze } = primordials;

/**
 * The builtin's namespace: `args`, the script's arguments as a frozen array of strings, and the
 * `default` every builtin has, `undefined`.
 */
export const create_args_namespace = () =>
	create_record({ args: ObjectFreeze(op_gro_args()), default: undefined });
