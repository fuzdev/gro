/**
 * Whether `error` is an instance of a class named `name` or of a subclass of one,
 * by walking its prototype chain, since a module can load a different copy
 * of Gro's error classes, which `instanceof` misses.
 */
export const is_error_class_named = (error: unknown, name: string): boolean => {
	if (typeof error !== 'object' || error === null) return false;
	for (let p = Object.getPrototypeOf(error); p !== null; p = Object.getPrototypeOf(p)) {
		if (Object.hasOwn(p, 'constructor') && p.constructor?.name === name) return true;
	}
	return false;
};
