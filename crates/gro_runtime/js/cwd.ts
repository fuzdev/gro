// The cwd builtin, `@fuzdev/gro/cwd.ts` on the runtime: the cwd host module's runtime
// implementation, the working directory the run was started in. The Node implementation,
// `src/lib/cwd.ts`, is the oracle, and its `cwd` is the one declaration of the type: this module
// declares none, and a test in `src/test` checks the namespace's inferred type against it.
//
// The directory is the host's, read once, when the builtin is first imported: `cwd` returns
// that string whenever it's called, and a run has no way to change its directory. As in the
// Node implementation, `cwd` is a frozen function that takes no receiver. When the directory's
// path isn't UTF-8, which the host checks, `cwd` throws when it's called, where importing the
// Node module throws: a factory never throws, since the import of a builtin whose factory
// threw would reject with the host's report of it, not the error, and never run it again.
//
// Evaluated in the snapshot, before any user code, so what it captures at its top level is the
// realm's own (`args.ts` has the rule), and the factory, which can run after user code has
// patched the global intrinsics, reaches none of them. It takes the scope of its importer's
// grant as its one argument, and a grant-free module has none, so its scope is `undefined`.
//
// This file must stay 7-bit ASCII: the build fails otherwise.

import { primordials } from 'ext:core/mod.js';
import { op_gro_cwd } from 'ext:core/ops';
import { create_record } from 'ext:gro/internal/records.ts';

const { Error, ObjectFreeze } = primordials;

/**
 * The builtin's namespace: `cwd`, a frozen function returning the working directory the run
 * was started in, or throwing an `Error` when its path isn't UTF-8, and the `default` every
 * builtin has, `undefined`.
 * @param _scope - the scope of the importer's grant, which a grant-free module has none of
 */
export const create_cwd_namespace = (_scope: undefined) => {
	const checked_cwd = op_gro_cwd();
	const cwd = (): string => {
		if (checked_cwd === null) {
			throw new Error("the working directory's path isn't UTF-8, so Gro can't name it");
		}
		return checked_cwd;
	};
	ObjectFreeze(cwd);
	return create_record({ cwd, default: undefined });
};
