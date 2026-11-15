// The blake3 builtin, `@fuzdev/gro/blake3.ts` on the runtime: the blake3 host module's runtime
// implementation, BLAKE3 hashing over bytes, by the native `blake3` crate. The Node
// implementation, `src/lib/blake3.ts`, is the oracle, and its exports and `Blake3Hasher` are the
// one declaration of the types: the hasher's type here is a claim, since the host makes the
// hasher, and a test in `src/test` checks the namespace's inferred type against the Node
// module's.
//
// Each export is a shell: a frozen function that takes no receiver and hands what it was
// called with to its host op, whole. The host reads bytes as a `Uint8Array` and a context as a
// string, natively, with no conversion, so no code of a caller's runs, and throws a
// `TypeError` for anything else. A hasher is the host's: an object it makes of the prototype
// this script gives, whose state is the host's own `blake3::Hasher`, which the host drops once
// the hasher is collected. The prototype is a frozen null-prototype record of shells, each
// handing its receiver and its arguments to an op of this row, whole: the host checks the
// receiver is a hasher it made. The script reads nothing and decides nothing.
//
// A script the runtime compiles and calls on the builtin's first import, in two levels,
// reaching intrinsics only through `primordials` (`args.ts` has the rules). The outer level
// makes the shells and the prototype, one of each for every importer, so a hasher one importer
// made is a hasher in another's hands. A grant-free module has no grant to scope, so its scope
// is `undefined`.
//
// This file must stay 7-bit ASCII: the build fails otherwise.

(function (bootstrap: Bootstrap) {
	const { core, primordials } = bootstrap;
	const {
		op_gro_blake3_derive_key_hasher,
		op_gro_blake3_finalize,
		op_gro_blake3_hash,
		op_gro_blake3_hasher,
		op_gro_blake3_keyed_hasher,
		op_gro_blake3_update
	} = core.ops;
	const { ObjectAssign, ObjectCreate, ObjectFreeze } = primordials;

	// a hasher's type, the one claim this script makes, held to the Node module's
	// `Blake3Hasher` by the types test
	interface HasherHandle {
		update(data: Uint8Array): void;
		finalize(): Uint8Array;
	}

	// the hashers' prototype, with no `constructor`: each method a shell of its op, which the
	// host freezes, as the prototype is, since every holder of a hasher shares them
	const prototype: HasherHandle = ObjectFreeze(
		ObjectAssign(ObjectCreate(null), {
			update(data: Uint8Array): void {
				op_gro_blake3_update(this, data);
			},
			finalize(): Uint8Array {
				return op_gro_blake3_finalize(this);
			}
		})
	);

	const hash = (data: Uint8Array): Uint8Array => op_gro_blake3_hash(data);
	ObjectFreeze(hash);
	const create_hasher = (): HasherHandle => op_gro_blake3_hasher(prototype);
	ObjectFreeze(create_hasher);
	const create_keyed_hasher = (key: Uint8Array): HasherHandle =>
		op_gro_blake3_keyed_hasher(prototype, key);
	ObjectFreeze(create_keyed_hasher);
	const create_derive_key_hasher = (context: string): HasherHandle =>
		op_gro_blake3_derive_key_hasher(prototype, context);
	ObjectFreeze(create_derive_key_hasher);

	// the one-shot hash and the hashers' factories, one of each for every importer, and the
	// `default` every builtin has, `undefined`
	return (_scope: undefined) =>
		ObjectAssign(ObjectCreate(null), {
			hash,
			create_hasher,
			create_keyed_hasher,
			create_derive_key_hasher,
			default: undefined
		});
});
