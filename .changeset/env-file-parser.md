---
'@fuzdev/gro': minor
---

feat: Gro parses env files itself, with `parse_env_file` in `@fuzdev/gro/env_files.ts`, which `load_env`, `load_from_env`, and `load_env_file` read through: it reads dotenv's grammar, that of its `parse` (`KEY=value` lines, `export `, comments, `'`, `"`, and `` ` `` quotes that may span lines, `\n` and `\r` in `"` quotes), expands no variable, and returns the vars or the first refusal and its line, for a quoted value the line its quote opened on. Vite 8 reads a SvelteKit app's env files with Node's `util.parseEnv` and dotenv-expand, so a value can differ between the app and Gro, `$` expansion most of all

**breaking** What dotenv reads silently in a way the file's author likely didn't mean is refused, and `load_env_file`, `load_env`, and `load_from_env` throw a `TaskError` naming the file and line, so `gro dev`'s server plugin, `gro publish`, and the `$app/env/*` shims fail on it: a line that isn't blank, a comment, or `KEY=value` (`KEY: value`, a bare `KEY`, a quoted value starting on the line after its key), an unquoted `#` not after whitespace (`URL=https://example.com/#a` read `https://example.com/`: quote the value), text after a closing quote but a comment, a quote with no closing one, a `\` right before a closing quote, the key `__proto__`, and U+2028 or U+2029 outside quotes.
