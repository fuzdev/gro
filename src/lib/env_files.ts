import type { EnvVarConfig } from '@sveltejs/kit/env';
import type { Result } from '@fuzdev/fuz_util/result.ts';
import { isAbsolute, join } from 'node:path';

import {
	env_var_has_visibility,
	type EnvVarDeclarations,
	type EnvVisibility,
	type LoadEnvOptions
} from './env_declarations.ts';
import type { FsPaths } from './fs_paths.ts';
import { TaskError } from './task_error.ts';

/**
 * Loads every env var from the env files plus the ambient env, later sources winning,
 * so the ambient env overrides the files.
 * A missing env file is skipped; one that can't be read throws.
 * @param fs_paths - reads the env files, a relative `env_dir` or file resolved against its project
 * @returns all loaded vars, unfiltered - `resolve_env_vars` picks out the declared ones
 * @throws `FsError` - when reading an env file fails other than by its absence
 * @throws `TaskError` - when an env file doesn't parse (`parse_env_file`)
 */
export const load_env = async (
	fs_paths: FsPaths,
	{ dev, env_dir, env_files = to_default_env_files(dev), ambient_env }: LoadEnvOptions
): Promise<Record<string, string>> => {
	const env: Record<string, string> = {};
	const loaded_files = await Promise.all(
		env_files.map((path) =>
			load_env_file(
				fs_paths,
				env_dir === undefined || isAbsolute(path) ? path : join(env_dir, path)
			)
		)
	);
	for (const loaded of loaded_files) {
		if (loaded) Object.assign(env, loaded);
	}
	for (const key in ambient_env) {
		const value = ambient_env[key];
		if (value !== undefined) env[key] = value;
	}
	return env;
};

/**
 * The env files Vite loads for a mode, in precedence order, later files winning.
 * @see https://vite.dev/guide/env-and-mode.html#env-files
 */
export const to_default_env_files = (dev: boolean): Array<string> => {
	const mode = dev ? 'development' : 'production';
	return ['.env', '.env.local', `.env.${mode}`, `.env.${mode}.local`];
};

/**
 * Loads a single env value without merging it into the ambient env.
 * Searches `ambient_env`, then each of `paths` that exists, by default only the project's `.env` -
 * never a parent directory's, so one directory's `.env` isn't a secret channel to every repo under it.
 * Empty strings are semantically the same as `undefined` for more ergonomic fallbacks.
 * @param fs_paths - reads the env files, a relative path resolved against its project
 * @param ambient_env - a plain record a root builds, needing only `key`
 * @param paths - absolute, or relative to the project
 * @throws `FsError` - when reading one of `paths` fails other than by its absence
 * @throws `TaskError` - when one of `paths` read doesn't parse (`parse_env_file`)
 */
export const load_from_env = async (
	fs_paths: FsPaths,
	ambient_env: Readonly<Record<string, string | undefined>>,
	key: string,
	paths = ['.env']
): Promise<string | undefined> => {
	let v = ambient_env[key];
	if (v) return v;
	for (const path of paths) {
		const env = await load_env_file(fs_paths, path);
		v = env?.[key];
		if (v) return v;
	}
	return undefined;
};

/**
 * Loads one env file's vars, parsed by `parse_env_file`, without merging them into anything.
 * @param fs_paths - reads the file, a relative `path` resolved against its project
 * @param path - absolute, or relative to the project
 * @returns the file's vars, or `null` when the file doesn't exist
 * @throws `FsError` - when reading the file fails other than by its absence
 * @throws `TaskError` - when the file doesn't parse, naming the path and line
 */
export const load_env_file = async (
	fs_paths: FsPaths,
	path: string
): Promise<Record<string, string> | null> => {
	const opened = await fs_paths.open_path(path);
	const loaded = await opened.dir.read_text_if_exists(opened.path);
	if (loaded === null) return null;
	const parsed = parse_env_file(loaded);
	if (!parsed.ok) {
		throw new TaskError(`Invalid env file ${path}, line ${parsed.line}: ${parsed.message}`);
	}
	return parsed.value;
};

// a JS `\s` but the line breaks, the whitespace within a line
const ASSIGNMENT_MATCHER =
	/^[^\S\n\u2028\u2029]*(?:export[^\S\n\u2028\u2029]+)?([\w.-]+)[^\S\n\u2028\u2029]*=/;
const QUOTED_VALUE_END_MATCHER = /^[^\S\n\u2028\u2029]*(?:#.*)?$/;
const SEPARATOR_MATCHER = /[\u2028\u2029]/;
const WHITESPACE_MATCHER = /\s/;

/**
 * Parses the text of an env file into its vars, reading dotenv's grammar (its `parse`)
 * and refusing what dotenv reads in a way the file's author likely didn't mean, rather than guessing.
 * Vite reads a SvelteKit app's env files differently, with Node's `util.parseEnv` and
 * dotenv-expand, so a value can differ between the app and Gro, `$` expansion most of all,
 * which Gro doesn't do.
 *
 * Each line is blank, a comment starting with `#`, or `KEY=value`, optionally after `export `,
 * a key being ASCII letters, digits, `_`, `.`, and `-`, and a later line of a key winning.
 * A value is one of:
 *
 * - unquoted, ending at a `#` after whitespace, which begins a comment, and trimmed
 * - in `'`, `"`, or `` ` `` quotes, its content kept as written, whitespace included,
 *   which may span lines and be followed by a comment;
 *   in `"` quotes, `\n` and `\r` are a line feed and a carriage return,
 *   and no other escape is read, so `\` is kept as written
 *
 * No variable is expanded, and a CRLF or CR line break reads as LF, in values too.
 *
 * Refused, where dotenv reads something else silently:
 *
 * - a line that isn't one of those, like `KEY: value` or a bare `KEY`,
 *   which dotenv skips or joins with the next line
 * - an unquoted `#` not after whitespace, which dotenv reads as a comment,
 *   so `KEY=a#b` is `a` there
 * - text after a closing quote but a comment, and a quote with no closing one,
 *   which dotenv reads unquoted
 * - a `\` right before the closing quote, which dotenv reads as escaping it or not
 *   by what follows, keeping the `\` either way
 * - the key `__proto__`, which dotenv drops
 * - U+2028 or U+2029 outside quotes, which dotenv reads as a line break in some places
 *
 * @param text - the file's contents
 * @returns the vars, keyed in the order their keys first appear but for integer-like keys,
 * which a JS object puts first, as in dotenv's; or the first refusal and the line it's on,
 * for a quoted value the line its opening quote is on
 */
export const parse_env_file = (
	text: string
): Result<{ value: Record<string, string> }, { message: string; line: number }> => {
	const src = text.replace(/\r\n?/g, '\n');
	const value: Record<string, string> = {};
	let line = 1;
	let start = 0;
	while (start <= src.length) {
		const found = src.indexOf('\n', start);
		const end = found === -1 ? src.length : found;
		const line_text = src.slice(start, end);
		let next = end + 1;
		let next_line = line + 1;
		const trimmed = line_text.trim();
		if (trimmed !== '' && !trimmed.startsWith('#')) {
			const matched = ASSIGNMENT_MATCHER.exec(line_text);
			if (!matched) {
				return {
					ok: false,
					message: SEPARATOR_MATCHER.test(line_text)
						? 'U+2028 and U+2029 are refused outside quotes'
						: 'expected `KEY=value`, a comment, or a blank line',
					line
				};
			}
			const key = matched[1]!;
			if (key === '__proto__') {
				return { ok: false, message: 'the key `__proto__` is refused', line };
			}
			const rest = line_text.slice(matched[0].length);
			const unindented = rest.trimStart();
			const quote = unindented[0];
			if (quote === '"' || quote === "'" || quote === '`') {
				if (SEPARATOR_MATCHER.test(rest.slice(0, rest.length - unindented.length))) {
					return { ok: false, message: 'U+2028 and U+2029 are refused outside quotes', line };
				}
				const open = start + matched[0].length + (rest.length - unindented.length);
				const close = src.indexOf(quote, open + 1);
				if (close === -1) {
					return { ok: false, message: `the value's opening ${quote} has no closing one`, line };
				}
				// a refusal from here names the line the value opens on, where its mistake likely is,
				// as an unclosed quote reads on to the next value's opening one
				let content = src.slice(open + 1, close);
				for (let i = content.indexOf('\n'); i !== -1; i = content.indexOf('\n', i + 1)) {
					next_line++;
				}
				if (content.endsWith('\\')) {
					return {
						ok: false,
						message: `a \`\\\` before the closing ${quote} is ambiguous, quote the value another way`,
						line
					};
				}
				const close_found = src.indexOf('\n', close);
				const close_end = close_found === -1 ? src.length : close_found;
				const after = src.slice(close + 1, close_end);
				if (SEPARATOR_MATCHER.test(after)) {
					return {
						ok: false,
						message: 'U+2028 and U+2029 are refused outside quotes',
						line
					};
				}
				if (!QUOTED_VALUE_END_MATCHER.test(after)) {
					return {
						ok: false,
						message: `only a comment may follow the closing ${quote}`,
						line
					};
				}
				if (quote === '"') content = content.replace(/\\n/g, '\n').replace(/\\r/g, '\r');
				value[key] = content;
				next = close_end + 1;
			} else {
				if (SEPARATOR_MATCHER.test(rest)) {
					return { ok: false, message: 'U+2028 and U+2029 are refused outside quotes', line };
				}
				const hash = rest.indexOf('#');
				if (hash !== -1 && (hash === 0 || !WHITESPACE_MATCHER.test(rest[hash - 1]!))) {
					return {
						ok: false,
						message:
							'a `#` not after whitespace is ambiguous, quote the value or space the comment',
						line
					};
				}
				value[key] = (hash === -1 ? rest : rest.slice(0, hash)).trim();
			}
		} else if (SEPARATOR_MATCHER.test(line_text)) {
			return { ok: false, message: 'U+2028 and U+2029 are refused outside quotes', line };
		}
		start = next;
		line = next_line;
	}
	return { ok: true, value };
};

type EnvVarSchema = NonNullable<EnvVarConfig<any>['schema']>;
type StandardSchema = Exclude<EnvVarSchema, (value: string | undefined) => any>;

/**
 * Normalizes a function validator to a Standard Schema, like `defineEnvVars` does,
 * for declarations that didn't go through it. Standard Schemas can be callable (ArkType),
 * so a function is only a validator if it isn't one.
 */
const to_standard_schema = (schema: EnvVarSchema | undefined): StandardSchema | undefined => {
	if (schema === undefined || '~standard' in schema) return schema;
	return {
		'~standard': {
			version: 1,
			vendor: 'gro',
			validate: (value) => {
				try {
					const result = schema(value as string | undefined);
					// handed back so `resolve_env_vars` reports it as an unsupported async validator
					if (result instanceof Promise) return result;
					return { value: result };
				} catch (error) {
					return { issues: [{ message: error instanceof Error ? error.message : String(error) }] };
				}
			}
		}
	};
};

/**
 * Resolves the declared env vars of one visibility from `env`, validating each the way
 * SvelteKit does: a var with no `schema` must be present, and one with a `schema` gets its
 * Standard Schema result, so validators can transform values and make them optional.
 * @param variables - the declarations from the project's `src/env.ts`
 * @param visibility - which of the declared vars to resolve
 * @param env - the values to resolve them from, usually from `load_env`
 * @returns the resolved values keyed by var name
 * @throws listing every invalid var, so a misconfigured env fails all at once
 */
export const resolve_env_vars = (
	variables: EnvVarDeclarations,
	visibility: EnvVisibility,
	env: Record<string, string | undefined>
): Record<string, unknown> => {
	const resolved: Record<string, unknown> = {};
	const issues: Array<string> = [];

	for (const [name, config] of Object.entries(variables)) {
		if (!env_var_has_visibility(config, visibility)) continue;
		const value = env[name];
		const validator = to_standard_schema(config.schema);
		if (!validator) {
			if (value === undefined) {
				issues.push(
					`${name}: value is missing - declare a schema that accepts it to make it optional`
				);
			}
			resolved[name] = value;
			continue;
		}
		const result = validator['~standard'].validate(value);
		if (result instanceof Promise) {
			issues.push(`${name}: async validators are not supported`);
			continue;
		}
		if (result.issues) {
			issues.push(`${name}: ${result.issues.map((i) => i.message).join('; ')}`);
			continue;
		}
		resolved[name] = result.value;
	}

	if (issues.length) {
		throw Error(`Invalid ${visibility} env vars:\n${issues.map((i) => '- ' + i).join('\n')}`);
	}

	return resolved;
};
