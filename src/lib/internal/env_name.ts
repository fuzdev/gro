/**
 * The check of an environment variable's name, shared by the process and spawn host modules.
 * Imports nothing, so either can use it without linking the other.
 *
 * @module
 */

/**
 * A variable name: non-empty, well-formed, without NUL or `=`, none of which can name a variable.
 * @param what - how an error names the value
 * @throws `TypeError` - for anything else, its message never holding the text after a `=`
 */
export const check_env_name = (value: unknown, what: string): string => {
	if (typeof value !== 'string') throw new TypeError(`${what} must be a string`);
	if (value.length === 0) throw new TypeError(`${what} is empty`);
	if (value.includes('\0')) throw new TypeError(`${what} holds a NUL byte`);
	if (!value.isWellFormed()) throw new TypeError(`${what} isn't well-formed Unicode`);
	// no echo of the text, as what follows a `=` can be a value
	if (value.includes('=')) throw new TypeError(`${what} holds a \`=\``);
	return value;
};
