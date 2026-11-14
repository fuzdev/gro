// The runtime_version builtin, `@fuzdev/gro/runtime_version.ts` on the runtime: the
// runtime_version host module's runtime implementation, the runtime's name and version. The
// Node implementation, `src/lib/runtime_version.ts`, is the oracle, and its `runtime_version` is
// the one declaration of the type: this module declares none, and a test in `src/test` checks
// the namespace's inferred type against it.
//
// The string is the host's, the embedder's name for itself and its version (`fgro/0.1.0`),
// read once, when the builtin is first imported, and exported as data. It differs from the
// Node implementation's (`node/v24.14.0`) by design: it's opaque, compared for equality only.
//
// Evaluated in the snapshot, before any user code (`args.ts` has the rule). The factory takes
// the scope of its importer's grant as its one argument, and a grant-free module has none, so
// its scope is `undefined`.
//
// This file must stay 7-bit ASCII: the build fails otherwise.

import { op_gro_runtime_version } from 'ext:core/ops';
import { create_record } from 'ext:gro/internal/records.ts';

/**
 * The builtin's namespace: `runtime_version`, the runtime's name and version, and the
 * `default` every builtin has, `undefined`.
 * @param _scope - the scope of the importer's grant, which a grant-free module has none of
 */
export const create_runtime_version_namespace = (_scope: undefined) =>
	create_record({ runtime_version: op_gro_runtime_version(), default: undefined });
