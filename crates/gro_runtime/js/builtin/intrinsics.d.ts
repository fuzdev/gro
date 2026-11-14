// The only global declarations a builtin's implementation is type-checked with, in place of
// TypeScript's own libraries (`noLib`): the types the compiler itself asks for and the ones
// `bootstrap.d.ts` names, each with no member, and no value at all.
//
// An implementation can run after user code has patched the global intrinsics, so it must name
// no global and read no property of an intrinsic. Here both are type errors: a global such as
// `Object` or `JSON` isn't declared, and `array.map`, `array.length`, `fn.call`, and
// `string.slice` aren't members. An implementation that needs one takes the function from
// `primordials` (`bootstrap.d.ts`). A member added here is a member every implementation may
// read after user code ran, so none is, and `Promise` and the iterator types stay undeclared,
// which makes `async`, `import()`, and a generator errors too.
//
// What a type check can't see stays a rule the tests hold by running the builtins under
// patched intrinsics: iterating (`for`-`of`, spread, both of which read `Symbol.iterator`),
// `instanceof`, a string made of an object, and a property read or written on an object whose
// prototype is `Object.prototype`.

interface Array<T> {}
interface Boolean {}
interface CallableFunction {}
interface Function {}
interface IArguments {}
interface NewableFunction {}
interface Number {}
interface Object {}
interface RegExp {}
interface String {}

interface ReadonlyArray<T> {}
type Readonly<T> = { readonly [K in keyof T]: T[K] };

interface Error {}
interface ErrorConstructor {
	new (message?: string): Error;
}
