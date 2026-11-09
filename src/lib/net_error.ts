/**
 * The errors of the net host module (`net.ts`), apart from it.
 *
 * Making or matching an error reaches no network, so this module grants nothing:
 * code that only classifies a failure, or builds one, imports it without linking the
 * host module, and a mock of the host module throws the same errors it does.
 * The class and its prototype are frozen.
 *
 * @module
 */

import { is_error_class_named } from './internal/error_class.ts';

/**
 * Why a request was refused. `not_granted` is a host outside what the handle allows, or a
 * scheme other than `http:` and `https:`, on the first request or a redirect.
 * A failure of the network itself is a `TypeError`, as from any `fetch`.
 */
export type NetErrorKind = 'not_granted';

/**
 * The error `net.ts` throws when a request is refused before it's sent, typed by `kind`.
 * Match it with `is_net_error`, which also recognizes one from another copy of Gro.
 * The class and its prototype are frozen.
 */
export class NetError extends Error {
	override name = 'NetError';
	readonly kind: NetErrorKind;
	/**
	 * What was refused: a host as `hostname:port`, or a scheme like `data:`.
	 * Never the whole URL, which can carry a token.
	 */
	readonly target: string;

	constructor(kind: NetErrorKind, target: string, message?: string, options?: ErrorOptions) {
		super(message ?? `${kind}: ${target}`, options);
		this.kind = kind;
		this.target = target;
	}
}

Object.freeze(NetError.prototype);
Object.freeze(NetError);

/**
 * Whether `error` is a `NetError` from any copy of Gro, optionally of one `kind`.
 * @param error - any thrown value
 * @param kind - when given, the kind the error must have
 */
export const is_net_error = (error: unknown, kind?: NetErrorKind): error is NetError =>
	is_error_class_named(error, 'NetError') &&
	(kind === undefined || (error as NetError).kind === kind);
