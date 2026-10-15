---
'@fuzdev/gro': minor
---

**breaking** feat: Gro no longer uses zod - `BuildCacheMetadata`, `BuildOutputEntry`, and `GithubPullRequest` are `@fuzdev/fuz_schema` schemas, and Gro reads package.json with the fuz_schema `PackageJson` of `@fuzdev/fuz_util` `0.74.0`

To upgrade:

- install `@fuzdev/fuz_util` `>=0.74.0` and `@fuzdev/fuz_schema` `^0.2.2`, the new peer floors - Gro 0.217 and earlier don't start with fuz_util 0.74
- `zod` is no longer a peer dep: add it to your own dependencies if your code imports it
- call `s.parse_or_throw(BuildCacheMetadata, value)` where you called a zod method like `BuildCacheMetadata.parse(value)`, and `s.parse` for `.safeParse`, likewise for `BuildOutputEntry` and `GithubPullRequest`

Changed:

- a package.json that fails to parse lists each issue as fuz_schema's `format_issue` text, like `dependencies.x: expected string, received number`, instead of zod's, like `"dependencies,x" Invalid input: expected string, received number`
- an issue inside a union nested in a union names the outer key and the member kinds, like `author: expected string | union, received object` for a bad `author.email` or `author.url`, where zod named the inner key, like `"author,url" Invalid URL`
- package.json writes (`gro sync`, `package_json_update`, `package_json_serialize`) keep the key order of nested objects like `repository`, `bugs`, `author`, and each `contributors` entry, and no longer trim the whitespace around a URL, where zod reordered and trimmed them; the other differences of fuz_util 0.74's `PackageJson` from zod's are in fuz_util's changelog
- `library_load_from_repo` returns the repo's `package_json` with its keys in the file's order, nested ones too, as is the `repository` in its `library_json`'s `pkg_json`, where zod put them in the schema's order - a result read from its cache keeps the old order until the repo's commit changes
- `library_load_from_repo` and `github_fetch_commit_prs` throw fuz_schema's `SchemaError` for invalid data, in place of a `ZodError`
- `gro run`, given args by `invoke_task` (the CLI's argv makes no bad forwarded arg), reports every forwarded arg that isn't a string, number, boolean, or array of those, after any bad `_` items, where it reported only the bad `_` items, or else only the first bad arg
