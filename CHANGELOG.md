# @fuzdev/gro

## 0.197.2

### Patch Changes

- fix publish task `access` arg ([b6f3035](https://github.com/fuzdev/gro/commit/b6f3035))

## 0.197.1

### Patch Changes

- add args to `gro release` ([ebc372e](https://github.com/fuzdev/gro/commit/ebc372e))

## 0.197.0

### Minor Changes

- switch to blake3 hashing, add peer dep `@fuzdev/blake3_wasm` ([#602](https://github.com/fuzdev/gro/pull/602))
- remove unused `EXPORTS_EXCLUDER_DEFAULT` ([51ae165](https://github.com/fuzdev/gro/commit/51ae165))
- upgrade fuz_util peer dep ([dac067d](https://github.com/fuzdev/gro/commit/dac067d))

### Patch Changes

- improve `find_cli` to use `command -v` instead of `which` ([643abf4](https://github.com/fuzdev/gro/commit/643abf4))

## 0.196.0

### Minor Changes

- upgrade fuz_util ([41fa74d](https://github.com/fuzdev/gro/commit/41fa74d))

## 0.195.2

### Patch Changes

- add `flags` to `gro_plugin_deno_server` ([58e2e96](https://github.com/fuzdev/gro/commit/58e2e96))

## 0.195.1

### Patch Changes

- add `gro_plugin_deno_server.ts` and `gro_plugin_deno_compile.ts` ([0b45163](https://github.com/fuzdev/gro/commit/0b45163))

## 0.195.0

### Minor Changes

- upgrade `fuz_util` ([#596](https://github.com/fuzdev/gro/pull/596))

## 0.194.0

### Minor Changes

- dont ignore test/ and benchmark/ dirs ([628fa71](https://github.com/fuzdev/gro/commit/628fa71))

### Patch Changes

- upgrade fuz_util ([df0f222](https://github.com/fuzdev/gro/commit/df0f222))

## 0.193.0

### Minor Changes

- upgrade fuz_util ([a9dff0a](https://github.com/fuzdev/gro/commit/a9dff0a))

### Patch Changes

- fix builtin specifier handling in filer ([a9dff0a](https://github.com/fuzdev/gro/commit/a9dff0a))

## 0.192.1

### Patch Changes

- add `allow_dirty` prop to build task ([6d402df](https://github.com/fuzdev/gro/commit/6d402df))

## 0.192.0

### Minor Changes

- transfer to @fuzdev/gro from @ryanatkn/gro ([#595](https://github.com/fuzdev/gro/pull/595))

### Patch Changes

- upgrade zod ([256ee15](https://github.com/fuzdev/gro/commit/256ee15))

## 0.191.0

### Minor Changes

- upgrade fuz_util ([1c43e8e](https://github.com/fuzdev/gro/commit/1c43e8e))

### Patch Changes

- error in CI for TODO markdown files ([0a5bc50](https://github.com/fuzdev/gro/commit/0a5bc50))

## 0.190.1

### Patch Changes

- add `filer_options` to `GroConfig` ([0cae1c6](https://github.com/fuzdev/gro/commit/0cae1c6))

## 0.190.0

### Minor Changes

- rename `DevTaskContext` from `Dev_Task_Context` ([d9575f8](https://github.com/fuzdev/gro/commit/d9575f8))
- upgrade fuz_util ([#593](https://github.com/fuzdev/gro/pull/593))
- upgrade fuz_util and use its `hash.ts` module ([#593](https://github.com/fuzdev/gro/pull/593))

### Patch Changes

- exclude test and benchmark dirs from search ([#590](https://github.com/fuzdev/gro/pull/590))

## 0.189.3

### Patch Changes

- improve `gro format` to take rest and forwarded args ([c9de33f](https://github.com/fuzdev/gro/commit/c9de33f))

## 0.189.2

### Patch Changes

- add `--build` and `--force_build` flags to `gro check` ([2557e50](https://github.com/fuzdev/gro/commit/2557e50))

## 0.189.1

### Patch Changes

- fix `gro run script.ts -- --help` to pass args after `--` to the script ([f05a9d8](https://github.com/fuzdev/gro/commit/f05a9d8))

## 0.189.0

### Minor Changes

- rework args ([#592](https://github.com/fuzdev/gro/pull/592))

## 0.188.0

### Minor Changes

- upgrade fuz_util ([63fb148](https://github.com/fuzdev/gro/commit/63fb148))

## 0.187.0

### Minor Changes

- upgrade fuz_util ([#591](https://github.com/fuzdev/gro/pull/591))

## 0.186.0

### Minor Changes

- update fuz_util ([#586](https://github.com/fuzdev/gro/pull/586))

### Patch Changes

- fix aliases in loader ([#586](https://github.com/fuzdev/gro/pull/586))

## 0.185.0

### Minor Changes

- use content hash for change detection, ignoring mtime ([#587](https://github.com/fuzdev/gro/pull/587))

## 0.184.0

### Minor Changes

- remove `load_package_json` for `package_json_load` ([e2594cb](https://github.com/fuzdev/gro/commit/e2594cb))
- rename some interfaces ([0375266](https://github.com/fuzdev/gro/commit/0375266))

## 0.183.0

### Minor Changes

- use fuz_ui vite plugin and remove related functionality ([#585](https://github.com/fuzdev/gro/pull/585))
- remove `include_nojekyll`, add `.nojekyll` manually to ./static ([79c5b3c](https://github.com/fuzdev/gro/commit/79c5b3c))

## 0.182.0

### Minor Changes

- replace `gro_plugin_sveltekit_app` option `host_target` with `include_nojekyll` ([751adf4](https://github.com/fuzdev/gro/commit/751adf4))

## 0.181.0

### Minor Changes

- migrate to fuzdev deps

## 0.180.0

### Minor Changes

- change to async IO ([#583](https://github.com/fuzdev/gro/pull/583))
- upgrade belt ([#583](https://github.com/fuzdev/gro/pull/583))

### Patch Changes

- improve `Filer` internals ([#583](https://github.com/fuzdev/gro/pull/583))

## 0.179.0

### Minor Changes

- upgrade belt and chokidar@5 ([627b6c2](https://github.com/fuzdev/gro/commit/627b6c2))

## 0.178.0

### Minor Changes

- rename `source_json.ts` from `src_json.ts` ([#582](https://github.com/fuzdev/gro/pull/582))

## 0.177.1

### Patch Changes

- fix args forwarding to `gro run` ([56bc239](https://github.com/fuzdev/gro/commit/56bc239))

## 0.177.0

### Minor Changes

- rename `PascalCase` from `Upper_Snake_Case` (lol) ([#581](https://github.com/fuzdev/gro/pull/581))

## 0.176.0

### Minor Changes

- change `gro run` to automatically forward args ([488f139](https://github.com/fuzdev/gro/commit/488f139))

## 0.175.0

### Minor Changes

- remove `fs.ts` and make some fs-reading APIs async ([#580](https://github.com/fuzdev/gro/pull/580))

### Patch Changes

- use >= for belt peer dep ([#580](https://github.com/fuzdev/gro/pull/580))

## 0.174.2

### Patch Changes

- fix `$app/` resolution in loader for node_modules ([82756d3](https://github.com/fuzdev/gro/commit/82756d3))

## 0.174.1

### Patch Changes

- install after version bump when deploying ([f9a7c95](https://github.com/fuzdev/gro/commit/f9a7c95))

## 0.174.0

### Minor Changes

- unpublish `package.ts` and `package.gen.ts` ([#578](https://github.com/fuzdev/gro/pull/578))

### Patch Changes

- improve filer missing file handling ([#579](https://github.com/fuzdev/gro/pull/579))
- add sync and install flags to publish task ([0629354](https://github.com/fuzdev/gro/commit/0629354))

## 0.173.2

### Patch Changes

- fix publish task to run gen ([#577](https://github.com/fuzdev/gro/pull/577))

## 0.173.1

### Patch Changes

- fix publish to sync first ([#576](https://github.com/fuzdev/gro/pull/576))

## 0.173.0

### Minor Changes

- upgrade belt ([#575](https://github.com/fuzdev/gro/pull/575))
- upgrade belt and migrate logger ([#575](https://github.com/fuzdev/gro/pull/575))

## 0.172.0

### Minor Changes

- move fixtures to src/test ([#573](https://github.com/fuzdev/gro/pull/573))

## 0.171.0

### Minor Changes

- add build cache ([#570](https://github.com/fuzdev/gro/pull/570))

## 0.170.0

### Minor Changes

- update peer deps ([963f1de](https://github.com/fuzdev/gro/commit/963f1de))

### Patch Changes

- support vitest@4 ([13d8499](https://github.com/fuzdev/gro/commit/13d8499))
- upgrade zod ([13d8499](https://github.com/fuzdev/gro/commit/13d8499))

## 0.169.1

### Patch Changes

- add `to_implicit_forwarded_args` to support better args forwarding for `gro test` ([#568](https://github.com/fuzdev/gro/pull/568))

## 0.169.0

### Minor Changes

- upstream git.ts and path.ts to belt ([#565](https://github.com/fuzdev/gro/pull/565))

### Patch Changes

- support ts imports ([#567](https://github.com/fuzdev/gro/pull/567))

## 0.168.0

### Minor Changes

- rename `create_src_modules` from `to_src_modules` ([2c5f99e](https://github.com/fuzdev/gro/commit/2c5f99e))

### Patch Changes

- ignore `.tmp.` files in `watch_dir` by default ([#566](https://github.com/fuzdev/gro/pull/566))
- add optional `log` param to `create_src_json` and `create_src_modules` ([2c5f99e](https://github.com/fuzdev/gro/commit/2c5f99e))

## 0.167.1

### Patch Changes

- support `null` as return value from `GenDependenciesResolver` ([1955bb5](https://github.com/fuzdev/gro/commit/1955bb5))

## 0.167.0

### Minor Changes

- add `changed_file_id` to gen context for dependency resolution ([#563](https://github.com/fuzdev/gro/pull/563))

### Patch Changes

- fix gen dependency cache busting ([#563](https://github.com/fuzdev/gro/pull/563))

## 0.166.1

### Patch Changes

- resolve `files` of gen dependencies ([#562](https://github.com/fuzdev/gro/pull/562))

## 0.166.0

### Minor Changes

- fix gen watch and improve the gen watch API with manual controls ([#561](https://github.com/fuzdev/gro/pull/561))

## 0.165.1

### Patch Changes

- remove debug logging from gen ([#560](https://github.com/fuzdev/gro/pull/560))

## 0.165.0

### Minor Changes

- switch to subpath patterns from explicit module listings in package.json ([#558](https://github.com/fuzdev/gro/pull/558))

## 0.164.1

### Patch Changes

- fix moss upgrade ([92dc22d](https://github.com/fuzdev/gro/commit/92dc22d))

## 0.164.0

### Minor Changes

- rework `Filer` API ([#557](https://github.com/fuzdev/gro/pull/557))

### Patch Changes

- add filer, log, timings, and invoke_task to `GenContext` ([#557](https://github.com/fuzdev/gro/pull/557))

## 0.163.0

### Minor Changes

- upgrade deps ([b9a5727](https://github.com/fuzdev/gro/commit/b9a5727))

### Patch Changes

- shim `asset` for `$app/paths` ([ac5be5b](https://github.com/fuzdev/gro/commit/ac5be5b))

## 0.162.2

### Patch Changes

- fix `gro test` to exit gracefully if there are no test files, opt out with arg `--fail_without_tests` ([#555](https://github.com/fuzdev/gro/pull/555))

## 0.162.1

### Patch Changes

- add no-pull option to `gro deploy` ([a56eb3f](https://github.com/fuzdev/gro/commit/a56eb3f))

## 0.162.0

### Minor Changes

- rename `cook_gro_config` from `normalize_gro_config` ([#553](https://github.com/fuzdev/gro/pull/553))
- change fn signature of `package_json_update` ([#552](https://github.com/fuzdev/gro/pull/552))
- move `SourceFile` from filer.ts and rename to `Disknode` in disknode.ts ([#553](https://github.com/fuzdev/gro/pull/553))
- rearrange some constants ([#551](https://github.com/fuzdev/gro/pull/551))
- - add `vitest` support ([#551](https://github.com/fuzdev/gro/pull/551))
  - remove `uvu` support
  - fail `gro test` if no test runner installed
- add @sveltejs/kit as peer dep ([#553](https://github.com/fuzdev/gro/pull/553))
- feat: change sync task to not install by default ([#549](https://github.com/fuzdev/gro/pull/549))
- invert the `write` boolean arg of `package_json_sync` from `check` ([#552](https://github.com/fuzdev/gro/pull/552))

### Patch Changes

- support NO_COLOR env var ([#551](https://github.com/fuzdev/gro/pull/551))

## 0.161.2

### Patch Changes

- fix another bug with belt package generation ([430902c](https://github.com/fuzdev/gro/commit/430902c))

## 0.161.1

### Patch Changes

- upgrade deps ([da81662](https://github.com/fuzdev/gro/commit/da81662))
- fix package generation for belt ([da81662](https://github.com/fuzdev/gro/commit/da81662))

## 0.161.0

### Minor Changes

- extract `pkg.ts` to Fuz ([#550](https://github.com/fuzdev/gro/pull/550))
- upgrade zod@4 ([#550](https://github.com/fuzdev/gro/pull/550))
- remove `DeclarationKind`, use instead `import type {SrcModuleDeclarationKind} from '@fuzdev/fuz_util/src_json.js';` ([#550](https://github.com/fuzdev/gro/pull/550))

### Patch Changes

- support SvelteKit shim for `resolve` ([#550](https://github.com/fuzdev/gro/pull/550))

## 0.160.0

### Minor Changes

- chore: upgrade deps ([dc24c7c](https://github.com/fuzdev/gro/commit/dc24c7c))

### Patch Changes

- improve gen logging ([#548](https://github.com/fuzdev/gro/pull/548))

## 0.159.0

### Minor Changes

- remove moss optional dep version cap ([85ae474](https://github.com/fuzdev/gro/commit/85ae474))

## 0.158.0

### Minor Changes

- change `SrcModule` to make `declarations` optional, ([#547](https://github.com/fuzdev/gro/pull/547))
  and when generated replace any empty arrays with `undefined`

## 0.157.1

### Patch Changes

- bump moss up to 0.29 ([b81be84](https://github.com/fuzdev/gro/commit/b81be84))

## 0.157.0

### Minor Changes

- remove ts-morph dep and use the ts API instead ([#546](https://github.com/fuzdev/gro/pull/546))
  - add `typescript^5` peer dep
  - merge `svelte_helpers.ts` into `constants.ts`
  - add default declarations:
    - new: `'./package.json': {path: 'package.json', declarations: [{name: 'default', kind: 'json'}]},`
    - old: `'./package.json': {path: 'package.json', declarations: []},`
  - add `parse_exports.ts`
  - add `SrcModuleDeclarationKind`

- bump oxc-parser@0.68.1 from 0.67 ([#546](https://github.com/fuzdev/gro/pull/546))
  - also bump zod patch

## 0.156.0

### Minor Changes

- - require import attr for json, `with { type: "json" }` ([#545](https://github.com/fuzdev/gro/pull/545))
  - remove `JSON_MATCHER`

## 0.155.0

### Minor Changes

- use Node's builtin type stripping ([#544](https://github.com/fuzdev/gro/pull/544))
  - bump node@22.15 from 22.11
  - add `ts-blank-space` dep to preprocess `.svelte.ts` modules
  - internally use `.ts` extensions

## 0.154.0

### Minor Changes

- bump moss@0.28 from 0.27 ([c8fabed](https://github.com/fuzdev/gro/commit/c8fabed))

## 0.153.2

### Patch Changes

- use flavored instead of Zod brand for git origin and branch types ([ea0a768](https://github.com/fuzdev/gro/commit/ea0a768))

## 0.153.1

### Patch Changes

- bump moss@0.27.0 from 0.26.0 ([a5994bb](https://github.com/fuzdev/gro/commit/a5994bb))

## 0.153.0

### Minor Changes

- rework some of the config impl and extend the options ([#543](https://github.com/fuzdev/gro/pull/543))
  - rename "SvelteKitConfig" to "SvelteConfig"
    - `svelte_config` from `sveltekit_config`
    - `SVELTE_CONFIG_FILENAME` from `SVELTEKIT_CONFIG_FILENAME`

### Patch Changes

- fix npm lockfile bug in `gro upgrade` ([2b98bd4](https://github.com/fuzdev/gro/commit/2b98bd4))
- fix moss plugin default ([#543](https://github.com/fuzdev/gro/pull/543))

## 0.152.0

### Minor Changes

- shim $app/state and remove the shim for $app/stores ([#541](https://github.com/fuzdev/gro/pull/541))
- upgrade oxc-parser@0.67.0 from 0.63.0 ([#540](https://github.com/fuzdev/gro/pull/540))

### Patch Changes

- dont throw on failed module resolution in the filer ([#536](https://github.com/fuzdev/gro/pull/536))
- add `gro upgrade` options `delete_node_modules`/`node_modules_path` and `delete_lockfile`/`lockfile_path` ([#540](https://github.com/fuzdev/gro/pull/540))

## 0.151.2

### Patch Changes

- add moss optional dep ([#539](https://github.com/fuzdev/gro/pull/539))

## 0.151.1

### Patch Changes

- fix module resolution in the filer for arbitrary dirs ([#537](https://github.com/fuzdev/gro/pull/537))

## 0.151.0

### Minor Changes

- fix `Filer` bug with missing source files ([#535](https://github.com/fuzdev/gro/pull/535))
  - **break**: move `filter_dependents` to `filer.ts` from `gro_plugin_gen.ts`

## 0.150.1

### Patch Changes

- upgrade @fuzdev/fuz_util@0.30.2 from 0.30.1 ([bb285d6](https://github.com/fuzdev/gro/commit/bb285d6))

## 0.150.0

### Minor Changes

- upgrade oxc-parser@0.63 from 0.62 ([#532](https://github.com/fuzdev/gro/pull/532))

## 0.149.2

### Patch Changes

- fix sveltekit shim regexp ([37900e5](https://github.com/fuzdev/gro/commit/37900e5))

## 0.149.1

### Patch Changes

- upgrade @fuzdev/fuz_util@0.30.1 ([7df1504](https://github.com/fuzdev/gro/commit/7df1504))

## 0.149.0

### Minor Changes

- upgrade deps ([#528](https://github.com/fuzdev/gro/pull/528))
- require node >=22.11 ([#528](https://github.com/fuzdev/gro/pull/528))
- upgrade oxc-parser@0.62 from 0.36 ([#523](https://github.com/fuzdev/gro/pull/523))

### Patch Changes

- map `svelte-check` output paths to be relative to the cwd ([#526](https://github.com/fuzdev/gro/pull/526))

## 0.148.0

### Minor Changes

- upgrade `@fuzdev/fuz_util@0.29.0` from `0.28.0` ([#522](https://github.com/fuzdev/gro/pull/522))

## 0.147.1

### Patch Changes

- fix `Filer` imports to `$app` and `$env` and add `SVELTEKIT_GLOBAL_SPECIFIER` ([9acc3c9](https://github.com/fuzdev/gro/commit/9acc3c9))

## 0.147.0

### Minor Changes

- simplify module resolution to use Node builtins instead of custom Vite-like behavior ([#519](https://github.com/fuzdev/gro/pull/519))

## 0.146.2

### Patch Changes

- upgrade `@fuzdev/fuz_util@0.26.1` from `0.26.0` and `tslib@2.8.1` from `2.8.0` ([a0764b6](https://github.com/fuzdev/gro/commit/a0764b6))

## 0.146.1

### Patch Changes

- replace `es-module-lexer` with `oxc-parser` ([#518](https://github.com/fuzdev/gro/pull/518))

## 0.146.0

### Minor Changes

- extract `moss_helpers.ts` and lazy load the Moss plugin ([9caa4a8](https://github.com/fuzdev/gro/commit/9caa4a8))

## 0.145.0

### Minor Changes

- make `package_json` required to some APIs ([8df8362](https://github.com/fuzdev/gro/commit/8df8362))

### Patch Changes

- fix Moss change by removing `load_moss_plugin` ([6af4ecb](https://github.com/fuzdev/gro/commit/6af4ecb))

## 0.144.1

### Patch Changes

- fix moss plugin ([852dab0](https://github.com/fuzdev/gro/commit/852dab0))

## 0.144.0

### Minor Changes

- upgrade `@fuzdev/fuz_util0.26.0` from `0.25.3` ([#516](https://github.com/fuzdev/gro/pull/516))
- upgrade `esm-env@1.1.4` from `1.0.0` ([#517](https://github.com/fuzdev/gro/pull/517))
- change `Options` interface convention to fully qualify the identifer, so `Options` becomes `FilerOptions` etc ([#515](https://github.com/fuzdev/gro/pull/515))

### Patch Changes

- move moss plugin from moss ([#516](https://github.com/fuzdev/gro/pull/516))
- uprade `esm-env@1.1.4` fom 1.0.0 ([#516](https://github.com/fuzdev/gro/pull/516))
- fix and export `resolve_exported_value` ([#517](https://github.com/fuzdev/gro/pull/517))

## 0.143.3

### Patch Changes

- change `gro changeset` to not join rest args ([2e835b3](https://github.com/fuzdev/gro/commit/2e835b3))
- fix gen plugin to ignore externals ([5398a32](https://github.com/fuzdev/gro/commit/5398a32))

## 0.143.2

### Patch Changes

- update to use SECRET_GITHUB_API_TOKEN ([706faf1](https://github.com/fuzdev/gro/commit/706faf1))

## 0.143.1

### Patch Changes

- fix Svelte peer dep to upgrade to 5 ([23d22a1](https://github.com/fuzdev/gro/commit/23d22a1))

## 0.143.0

### Minor Changes

- add partial support for configurable js CLIs ([#514](https://github.com/fuzdev/gro/pull/514))
- rename `constants.ts` from `path_constants.ts` ([#514](https://github.com/fuzdev/gro/pull/514))

## 0.142.0

### Minor Changes

- fix `Filer` to resolve node modules ([#512](https://github.com/fuzdev/gro/pull/512))

### Patch Changes

- add `pm_cli` config ([#513](https://github.com/fuzdev/gro/pull/513))

## 0.141.1

### Patch Changes

- fix `parse_imports` for other file types ([c4c2fb0](https://github.com/fuzdev/gro/commit/c4c2fb0))

## 0.141.0

### Minor Changes

- change `Filer` to default to matching all files ([2616239](https://github.com/fuzdev/gro/commit/2616239))

## 0.140.7

### Patch Changes

- add `mtime` to `Disknode` ([becb071](https://github.com/fuzdev/gro/commit/becb071))

## 0.140.6

### Patch Changes

- add `root_dir` to `Filer` ([8374d05](https://github.com/fuzdev/gro/commit/8374d05))

## 0.140.5

### Patch Changes

- revert filer bug to undo support for moss optimizer ([768954f](https://github.com/fuzdev/gro/commit/768954f))
- support a local moss plugin path ([5aa2bf7](https://github.com/fuzdev/gro/commit/5aa2bf7))

## 0.140.4

### Patch Changes

- revert fix again lol ([4531de9](https://github.com/fuzdev/gro/commit/4531de9))

## 0.140.3

### Patch Changes

- revert broken filer fix ([#511](https://github.com/fuzdev/gro/pull/511))

## 0.140.2

### Patch Changes

- fix filer node_modules path resolution ([#510](https://github.com/fuzdev/gro/pull/510))

## 0.140.1

### Patch Changes

- fix filer to include node_modules ([#509](https://github.com/fuzdev/gro/pull/509))

## 0.140.0

### Minor Changes

- upgrade `ts-morph@24.0.0` from `23.0.0` ([bf448d2](https://github.com/fuzdev/gro/commit/bf448d2))

## 0.139.2

### Patch Changes

- rearrange default plugin to fix generated files with SvelteKit ([825908f](https://github.com/fuzdev/gro/commit/825908f))

## 0.139.1

### Patch Changes

- add moss plugin to default config ([#507](https://github.com/fuzdev/gro/pull/507))

## 0.139.0

### Minor Changes

- move `throttle` to `@fuzdev/fuz_util` and upgrade to `0.25.2` from `0.25.0` ([#505](https://github.com/fuzdev/gro/pull/505))

### Patch Changes

- upgrade belt ([942b1c0](https://github.com/fuzdev/gro/commit/942b1c0))

## 0.138.2

### Patch Changes

- cleanup `gro_plugin_gen` ([#504](https://github.com/fuzdev/gro/pull/504))

## 0.138.1

### Patch Changes

- add `leading` option to `throttle` ([#503](https://github.com/fuzdev/gro/pull/503))

## 0.138.0

### Minor Changes

- improve config types making `plugins` more usable ([#502](https://github.com/fuzdev/gro/pull/502))

## 0.137.0

### Minor Changes

- notify `Filer` changes only for files whose contents have changed ([498d4a6](https://github.com/fuzdev/gro/commit/498d4a6))

## 0.136.1

### Patch Changes

- add `input_paths` option to `gro_plugin_gen` ([2673241](https://github.com/fuzdev/gro/commit/2673241))

## 0.136.0

### Minor Changes

- bump minor ([79f5dfa](https://github.com/fuzdev/gro/commit/79f5dfa))

## 0.135.3

### Patch Changes

- feat: watch gen in `gro dev` ([#492](https://github.com/fuzdev/gro/pull/492))
- add `parse_imports.ts` ([#492](https://github.com/fuzdev/gro/pull/492))

## 0.135.2

### Patch Changes

- clean up `search_fs` docs ([a4884bf](https://github.com/fuzdev/gro/commit/a4884bf))

## 0.135.1

### Patch Changes

- upgrade `chokidar@4.0.1` from `4.0.0` ([58e324c](https://github.com/fuzdev/gro/commit/58e324c))

## 0.135.0

### Minor Changes

- bump required node version to `20.17` ([#498](https://github.com/fuzdev/gro/pull/498))

### Patch Changes

- use node's `styleText` directly instead of belt's re-exports ([#498](https://github.com/fuzdev/gro/pull/498))

## 0.134.0

### Minor Changes

- upgrade `chokidar@4` ([#496](https://github.com/fuzdev/gro/pull/496))

### Patch Changes

- fix server plugin to set the node condition for `esm-env` ([#497](https://github.com/fuzdev/gro/pull/497))

## 0.133.8

### Patch Changes

- fix typechecking for non-SvelteKit projects ([5470bf4](https://github.com/fuzdev/gro/commit/5470bf4))

## 0.133.7

### Patch Changes

- upgrade `@fuzdev/fuz_util@0.24.12` ([d1ae0e2](https://github.com/fuzdev/gro/commit/d1ae0e2))

## 0.133.6

### Patch Changes

- feat: support `?raw` imports in the loader ([#494](https://github.com/fuzdev/gro/pull/494))

## 0.133.5

### Patch Changes

- add `origin_path` to gen context ([998f99a](https://github.com/fuzdev/gro/commit/998f99a))
- fix: spawn server in `gro_plugin_server.ts` correctly ([c7fcf8b](https://github.com/fuzdev/gro/commit/c7fcf8b))

## 0.133.4

### Patch Changes

- fix: upgrade belt to exit on uncaught exceptions ([e1d1e9d](https://github.com/fuzdev/gro/commit/e1d1e9d))

## 0.133.3

### Patch Changes

- add `sync` and `no-sync` args to `gro build` ([9df775c](https://github.com/fuzdev/gro/commit/9df775c))
- fix the loader for `.svelte.ts` modules ([#491](https://github.com/fuzdev/gro/pull/491))

## 0.133.2

### Patch Changes

- tweak error handling ([#490](https://github.com/fuzdev/gro/pull/490))

## 0.133.1

### Patch Changes

- fix `peerDependenciesMeta` type ([9b3c8eb](https://github.com/fuzdev/gro/commit/9b3c8eb))

## 0.133.0

### Minor Changes

- remove `url` arg from `parse_package_meta` and require `package_json.homepage` ([#489](https://github.com/fuzdev/gro/pull/489))

### Patch Changes

- fix bug with `create_src_json` and similarly named files ([#489](https://github.com/fuzdev/gro/pull/489))

## 0.132.0

### Minor Changes

- rename `gro_config.ts` from `config.ts` ([#488](https://github.com/fuzdev/gro/pull/488))

## 0.131.0

### Minor Changes

- rename gro config helpers to include `gro_` for clarity ([#487](https://github.com/fuzdev/gro/pull/487))
  - rename `load_gro_config` from `load_config`
  - rename `cook_gro_config` from `normalize_config`
  - rename `create_empty_gro_config ` from `create_empty_config `
  - rename `validate_gro_config_module` from `validate_config_module`

## 0.130.2

### Patch Changes

- add `logo_url` and `logo_alt` to `package_meta` ([b3387d8](https://github.com/fuzdev/gro/commit/b3387d8))
- remove unicode flag from regexps ([ead9757](https://github.com/fuzdev/gro/commit/ead9757))

## 0.130.1

### Patch Changes

- build when publish is optional and there is no version change ([c93fb7b](https://github.com/fuzdev/gro/commit/c93fb7b))

## 0.130.0

### Minor Changes

- install by default in `gro sync` ([#486](https://github.com/fuzdev/gro/pull/486))
- rename `gro changeset` arg `dep` from `install` ([#486](https://github.com/fuzdev/gro/pull/486))

### Patch Changes

- add `install`/`no-install` args to `gro check` and `gro dev` ([#486](https://github.com/fuzdev/gro/pull/486))

## 0.129.15

### Patch Changes

- fix check to not sync ([#485](https://github.com/fuzdev/gro/pull/485))
- add `sync` and `no-sync` args to `gro check` ([#485](https://github.com/fuzdev/gro/pull/485))
- upgrade `@fuzdev/fuz_util@0.24.4` from `0.24.2` ([#485](https://github.com/fuzdev/gro/pull/485))

## 0.129.14

### Patch Changes

- improve task name inference ([12faa47](https://github.com/fuzdev/gro/commit/12faa47))

## 0.129.13

### Patch Changes

- upgrade `@fuzdev/fuz_util@0.24.4` ([4d46d33](https://github.com/fuzdev/gro/commit/4d46d33))

## 0.129.12

### Patch Changes

- fix tsconfig `sourceRoot` ([ba35389](https://github.com/fuzdev/gro/commit/ba35389))

## 0.129.11

### Patch Changes

- add tsconfig `sourceRoot` to fix declaration maps ([faceb82](https://github.com/fuzdev/gro/commit/faceb82))

## 0.129.10

### Patch Changes

- ignore dist test files ([4faa34e](https://github.com/fuzdev/gro/commit/4faa34e))

## 0.129.9

### Patch Changes

- add ignore pattern directly to files and delete .npmignore ([e1871b0](https://github.com/fuzdev/gro/commit/e1871b0))

## 0.129.8

### Patch Changes

- try again to fix .npmignore ([9b05272](https://github.com/fuzdev/gro/commit/9b05272))

## 0.129.7

### Patch Changes

- update .npmignore ([e567bb7](https://github.com/fuzdev/gro/commit/e567bb7))
- move docs to `src/` from `src/lib` ([ddd6d1b](https://github.com/fuzdev/gro/commit/ddd6d1b))

## 0.129.6

### Patch Changes

- attempt to fix .npmignore directories ([a051f49](https://github.com/fuzdev/gro/commit/a051f49))

## 0.129.5

### Patch Changes

- add .npmignore and include src/lib ([85a627a](https://github.com/fuzdev/gro/commit/85a627a))

## 0.129.4

### Patch Changes

- enable tsconfig `declaration` and disable `isolatedModules` because SvelteKit does it ([2a936af](https://github.com/fuzdev/gro/commit/2a936af))
- upgrade `@fuzdev/fuz_util@0.24.3` ([0b74b12](https://github.com/fuzdev/gro/commit/0b74b12))

## 0.129.3

### Patch Changes

- enable tsconfig `declarationMap` and `isolatedModules` ([a24fea2](https://github.com/fuzdev/gro/commit/a24fea2))

## 0.129.2

### Patch Changes

- fix bug inferring task names ([#484](https://github.com/fuzdev/gro/pull/484))
- improve `gro upgrade` to handle more version variants ([#483](https://github.com/fuzdev/gro/pull/483))

## 0.129.1

### Patch Changes

- upgrade `@fuzdev/fuz_util@0.24.2` from `0.24.1` ([a107801](https://github.com/fuzdev/gro/commit/a107801))

## 0.129.0

### Minor Changes

- change `resolve_gro_module_path` to be sync ([#481](https://github.com/fuzdev/gro/pull/481))

### Patch Changes

- add `esm-env` and set the export condition to development by default ([#481](https://github.com/fuzdev/gro/pull/481))

## 0.128.0

### Minor Changes

- upgrade `@fuzdev/fuz_util@0.24` and fix task timings ([9dc0a5f](https://github.com/fuzdev/gro/commit/9dc0a5f))

## 0.127.1

### Patch Changes

- add `sideEffects` to `package.json` ([b5fd793](https://github.com/fuzdev/gro/commit/b5fd793))
- support `logo` and `logo_alt` in `package.json` ([#480](https://github.com/fuzdev/gro/pull/480))

## 0.127.0

### Minor Changes

- upgrade `esbuild@0.21` ([#479](https://github.com/fuzdev/gro/pull/479))

## 0.126.0

### Minor Changes

- make `find_cli`, `spawn_cli_process`, and `resolve_cli` synchronous ([#478](https://github.com/fuzdev/gro/pull/478))

### Patch Changes

- rename and move some identifiers for consistency ([6513cd4](https://github.com/fuzdev/gro/commit/6513cd4))
- support svelte-package options in `gro_plugin_sveltekit_library` ([f7e2b72](https://github.com/fuzdev/gro/commit/f7e2b72))

## 0.125.1

### Patch Changes

- replace `kleur` with Node's `styleText` ([#477](https://github.com/fuzdev/gro/pull/477))

## 0.125.0

### Minor Changes

- improve CLI handling ([#475](https://github.com/fuzdev/gro/pull/475))
  - change `find_cli` to return an object
  - remove `npx` usage
  - add a bunch of CLI customizability
  - improve CLI resolution performance

- rename `package.json` `icon` to `glyph` ([#476](https://github.com/fuzdev/gro/pull/476))
- add `RawGroConfig` that's transformed to `GroConfig` via `normalized_config`, ([#473](https://github.com/fuzdev/gro/pull/473))
  changing usage after creation to be more strict but keeping user definitions relaxed

### Patch Changes

- rearrange generated exports ([#474](https://github.com/fuzdev/gro/pull/474))

## 0.124.0

### Minor Changes

- support `node@20.12` and later ([fcd3b7e](https://github.com/fuzdev/gro/commit/fcd3b7e))

## 0.123.0

### Minor Changes

- change `to_task_name` to accept a root path ([4d843e7](https://github.com/fuzdev/gro/commit/4d843e7))

### Patch Changes

- fix `resolve_input_files` corner case with multiples ([#472](https://github.com/fuzdev/gro/pull/472))

## 0.122.0

### Minor Changes

- remove `resolved_input_paths_by_input_path` and `resolved_input_files_by_input_path` ([#471](https://github.com/fuzdev/gro/pull/471))
- upgrade `@fuzdev/fuz_util@0.21` ([46a03e8](https://github.com/fuzdev/gro/commit/46a03e8))

### Patch Changes

- fix running `gro` with no args and no local tasks ([#471](https://github.com/fuzdev/gro/pull/471))

## 0.121.1

### Patch Changes

- remove options from `resolve_input_files` calling `search_fs` ([dd83f14](https://github.com/fuzdev/gro/commit/dd83f14))
- more search excluder fixes ([dd83f14](https://github.com/fuzdev/gro/commit/dd83f14))

## 0.121.0

### Minor Changes

- change some functions to be synchronous - `resolve_input_paths`, `resolve_input_files`, ([#470](https://github.com/fuzdev/gro/pull/470))
  `find_tasks`, `search_fs`

### Patch Changes

- fix the default search excluder ([#470](https://github.com/fuzdev/gro/pull/470))

## 0.120.1

### Patch Changes

- fix search options for tasks and genfiles ([f17187a](https://github.com/fuzdev/gro/commit/f17187a))

## 0.120.0

### Minor Changes

- rename `PathId` from `SourceId` ([#468](https://github.com/fuzdev/gro/pull/468))
- upgrade node@22.3 ([#469](https://github.com/fuzdev/gro/pull/469))
- rename `PathInfo` from `PathData` ([#469](https://github.com/fuzdev/gro/pull/469))
- rewrite task resolution ([#468](https://github.com/fuzdev/gro/pull/468))
- rewrite `search_fs` to not use globs ([#469](https://github.com/fuzdev/gro/pull/469))

### Patch Changes

- remove `exists`, use `existsSync` from `fs:node` instead ([#469](https://github.com/fuzdev/gro/pull/469))

## 0.119.1

### Patch Changes

- fix task name inference to node_modules ([#467](https://github.com/fuzdev/gro/pull/467))

## 0.119.0

### Minor Changes

- remove the `--install`/`--no-install` args to `gro deploy` and `gro publish`, use `gro deploy -- gro build --no-install` instead ([#464](https://github.com/fuzdev/gro/pull/464))
- improve errors detecting project features ([#463](https://github.com/fuzdev/gro/pull/463))
- change `gro build` default `install` to `true`, disable with `--no-install` ([#464](https://github.com/fuzdev/gro/pull/464))
- move `sveltekit_sync` to `$lib/sveltekit_helpers.ts` ([#462](https://github.com/fuzdev/gro/pull/462))
- write gen files only when changed ([#466](https://github.com/fuzdev/gro/pull/466))

### Patch Changes

- change `gro changeset` to commit and push only if all changes are staged ([bb1d25a](https://github.com/fuzdev/gro/commit/bb1d25a))
- add `--optional` arg to `gro publish` and use it from `gro release` ([dd02f27](https://github.com/fuzdev/gro/commit/dd02f27))
- add reinstall task ([#461](https://github.com/fuzdev/gro/pull/461))
- add `--only` option to `gro upgrade` ([99b0fb5](https://github.com/fuzdev/gro/commit/99b0fb5))
- run `svelte-kit sync` on startup if needed ([#462](https://github.com/fuzdev/gro/pull/462))
- add `--no-pull` option to `gro publish` ([f1b8fa7](https://github.com/fuzdev/gro/commit/f1b8fa7))

## 0.118.0

### Minor Changes

- cleanup task internals, renaming some modules and moving some helpers ([#460](https://github.com/fuzdev/gro/pull/460))
  - use SvelteKit config for lib and routes paths
  - print task names relative to the first match in `task_root_dirs`

### Patch Changes

- add the register hook `@fuzdev/gro/register.js` and document it in the readme ([#460](https://github.com/fuzdev/gro/pull/460))

## 0.117.0

### Minor Changes

- enable running tasks from outside src/lib ([#452](https://github.com/fuzdev/gro/pull/452))
  - add the config option `task_root_dirs`
  - input paths for tasks/gen now resolve explicitly relative paths `./foo` relative to the cwd
  - package.json is now optional

## 0.116.2

### Patch Changes

- fix `gro --help` for `ZodBranded` type ([#458](https://github.com/fuzdev/gro/pull/458))

## 0.116.1

### Patch Changes

- improve options forwarding for svelte module compilation ([ef49d46](https://github.com/fuzdev/gro/commit/ef49d46))

## 0.116.0

### Minor Changes

- upgrade to svelte@5 and esbuild@0.20 ([#454](https://github.com/fuzdev/gro/pull/454))

## 0.115.3

### Patch Changes

- feat: add --build flag to `gro publish` ([dd23fab](https://github.com/fuzdev/gro/commit/dd23fab))

## 0.115.2

### Patch Changes

- add `--no-pull` to `gro upgrade` ([975f9d3](https://github.com/fuzdev/gro/commit/975f9d3))

## 0.115.1

### Patch Changes

- add optional `tagline` property to `PackageJson` ([600143f](https://github.com/fuzdev/gro/commit/600143f))

## 0.115.0

### Minor Changes

- rename `well_known_src_files` from `well_known_src` to reduce ambiguity with `well_known_src_json` ([fddf9ad](https://github.com/fuzdev/gro/commit/fddf9ad))

## 0.114.0

### Minor Changes

- don't copy `src/` to `static/.well-known/` by default ([#456](https://github.com/fuzdev/gro/pull/456))
  - rename `filter_well_known_src` to `well_known_src_files`
  - change `well_known_src_files` to accept a boolean that defaults to `false`

### Patch Changes

- include `./package.json` in the `package.json#exports` ([#457](https://github.com/fuzdev/gro/pull/457))

## 0.113.1

### Patch Changes

- add `origin` to `gro publish` and brand GitBranch and GitOrigin ([028a509](https://github.com/fuzdev/gro/commit/028a509))
- add `origin` to `gro commit` options ([3c9bfe8](https://github.com/fuzdev/gro/commit/3c9bfe8))
- add `--force` flag to `gro upgrade` ([6a16004](https://github.com/fuzdev/gro/commit/6a16004))
- error on `gro changeset` if no lib detected ([ac55984](https://github.com/fuzdev/gro/commit/ac55984))
- add `origin` to `gro upgrade` and pull first ([1c171e1](https://github.com/fuzdev/gro/commit/1c171e1))

## 0.113.0

### Minor Changes

- upgrade deps ([c7521a6](https://github.com/fuzdev/gro/commit/c7521a6))
- remove `format_host` ([5ab1a33](https://github.com/fuzdev/gro/commit/5ab1a33))

## 0.112.5

### Patch Changes

- upgrade deps ([#453](https://github.com/fuzdev/gro/pull/453))

## 0.112.4

### Patch Changes

- upgrade @fuzdev/fuz_util ([3622e1d](https://github.com/fuzdev/gro/commit/3622e1d))

## 0.112.3

### Patch Changes

- upgrade deps ([04f2858](https://github.com/fuzdev/gro/commit/04f2858))

## 0.112.2

### Patch Changes

- add package meta ([#451](https://github.com/fuzdev/gro/pull/451))

## 0.112.1

### Patch Changes

- upgrade deps ([4bf736f](https://github.com/fuzdev/gro/commit/4bf736f))

## 0.112.0

### Minor Changes

- republish ([717be93](https://github.com/fuzdev/gro/commit/717be93))

## 0.111.1

### Patch Changes

- move util org ([a4095eb](https://github.com/fuzdev/gro/commit/a4095eb))

## 0.111.0

### Minor Changes

- move org to @ryanatkn from @grogarden ([#448](https://github.com/fuzdev/gro/pull/448))

## 0.110.2

### Patch Changes

- clean up type imports from schema removal ([4d3c8ee](https://github.com/fuzdev/gro/commit/4d3c8ee))

## 0.110.1

### Patch Changes

- cleanup schema removal ([1bb28cd](https://github.com/fuzdev/gro/commit/1bb28cd))

## 0.110.0

### Minor Changes

- remove json schema support for zod ([#447](https://github.com/fuzdev/gro/pull/447))

## 0.109.1

### Patch Changes

- fix flavored zod types ([3822041](https://github.com/fuzdev/gro/commit/3822041))
- add `gro run` task ([#446](https://github.com/fuzdev/gro/pull/446))

## 0.109.0

### Minor Changes

- upgrade @fuzdev/fuz_util@0.19 ([0f53451](https://github.com/fuzdev/gro/commit/0f53451))

### Patch Changes

- fix `gro changeset` commit message quotes ([88aaa95](https://github.com/fuzdev/gro/commit/88aaa95))

## 0.108.1

### Patch Changes

- support sourcemaps ([#445](https://github.com/fuzdev/gro/pull/445))

## 0.108.0

### Minor Changes

- rename `gro_plugin_sveltekit_app` from `gro_plugin_sveltekit_frontend` ([#444](https://github.com/fuzdev/gro/pull/444))
- rename `gro_plugin_sveltekit_library` from `gro_plugin_sveltekit_library` ([#444](https://github.com/fuzdev/gro/pull/444))

### Patch Changes

- fix the server plugin while building ([#444](https://github.com/fuzdev/gro/pull/444))

## 0.107.4

### Patch Changes

- support schema identifiers with underscores ([#443](https://github.com/fuzdev/gro/pull/443))

## 0.107.3

### Patch Changes

- upgrade @fuzdev/fuz_util ([dfcbe4c](https://github.com/fuzdev/gro/commit/dfcbe4c))

## 0.107.2

### Patch Changes

- upgrade @fuzdev/fuz_util ([4baf889](https://github.com/fuzdev/gro/commit/4baf889))

## 0.107.1

### Patch Changes

- fix SvelteKit esbuild plugin to include `resolveDir` ([32da839](https://github.com/fuzdev/gro/commit/32da839))

## 0.107.0

### Minor Changes

- upgrade @sveltejs/kit@2 with @esbuild@0.19 ([#442](https://github.com/fuzdev/gro/pull/442)) ([9b0082e](https://github.com/fuzdev/gro/commit/9b0082e))

## 0.106.0

### Minor Changes

- upgrade node@20.10 ([#441](https://github.com/fuzdev/gro/pull/441))

### Patch Changes

- use `--import` instead of `--loader` ([#441](https://github.com/fuzdev/gro/pull/441))
- add message and commit automation support to `gro changeset` ([#440](https://github.com/fuzdev/gro/pull/440))

## 0.105.5

### Patch Changes

- upgrade @fuzdev/fuz_util to use `fetch_value` ([#439](https://github.com/fuzdev/gro/pull/439))

## 0.105.4

### Patch Changes

- reset target branch on deploy ([#438](https://github.com/fuzdev/gro/pull/438))
- handle target branch changes on deploy ([#438](https://github.com/fuzdev/gro/pull/438))

## 0.105.3

### Patch Changes

- warn if no github token ([caf585c](https://github.com/fuzdev/gro/commit/caf585c))
- forward args from `create_src_json` ([edbc4c1](https://github.com/fuzdev/gro/commit/edbc4c1))

## 0.105.2

### Patch Changes

- finally fix issues with [#427](https://github.com/fuzdev/gro/pull/427) ([9a9b174](https://github.com/fuzdev/gro/commit/9a9b174))

## 0.105.1

### Patch Changes

- fix issues with [#427](https://github.com/fuzdev/gro/pull/427) ([c9dc961](https://github.com/fuzdev/gro/commit/c9dc961))

## 0.105.0

### Minor Changes

- add and integrate `gro changelog` ([#427](https://github.com/fuzdev/gro/pull/427))

## 0.104.2

### Patch Changes

- avoid building twice on `gro release` ([cbc4762](https://github.com/fuzdev/gro/commit/cbc4762))
  - if `gro publish` is called before `gro deploy`, we don't need to build to deploy

## 0.104.1

### Patch Changes

- changelog test ([f7f6ee9](https://github.com/fuzdev/gro/commit/f7f6ee9))

  this is just a test commit for testing purposes

  because
  - changelog
  - other things

  etc

## 0.104.0

### Minor Changes

- replace `exists.ts` with `fs.ts` and add `empty_dir` ([#429](https://github.com/fuzdev/gro/pull/429))
- stop using `git workspace` for `gro deploy` ([#429](https://github.com/fuzdev/gro/pull/429))
- upgrade deps: ([#437](https://github.com/fuzdev/gro/pull/437))
  - @fuzdev/fuz_util@18
  - prettier@3.1.1
  - ts-morph@21

### Patch Changes

- shim SvelteKit's `resolveRoute` ([5e94cd4](https://github.com/fuzdev/gro/commit/5e94cd4))

## 0.103.2

### Patch Changes

- fix gro build for deps that circularly import the building project ([e345eaa](https://github.com/fuzdev/gro/commit/e345eaa))

## 0.103.1

### Patch Changes

- improve `exists` ([094279d](https://github.com/fuzdev/gro/commit/094279d))

## 0.103.0

### Minor Changes

- upgrade deps ([f6133f7](https://github.com/fuzdev/gro/commit/f6133f7))

## 0.102.3

### Patch Changes

- upgrade deps ([54b65ec](https://github.com/fuzdev/gro/commit/54b65ec))

## 0.102.2

### Patch Changes

- catch server build errors ([80365d0](https://github.com/fuzdev/gro/commit/80365d0))

## 0.102.1

### Patch Changes

- clean sveltekit dist dir ([3d84dfd](https://github.com/fuzdev/gro/commit/3d84dfd))
- check for clean git workspace in `gro publish` ([fc64b77](https://github.com/fuzdev/gro/commit/fc64b77))

## 0.102.0

### Minor Changes

- add `$lib/src_json.ts` ([#434](https://github.com/fuzdev/gro/pull/434))
  - add `.well-known/src.json` and `.well-known/src/`
  - remove `modules` from `.well-known/package.json`

### Patch Changes

- modify exports by default only for libraries ([#434](https://github.com/fuzdev/gro/pull/434))

## 0.101.0

### Minor Changes

- improve modules type info ([#435](https://github.com/fuzdev/gro/pull/435))

## 0.100.4

### Patch Changes

- change `.well-known/package.json` and `.nojekyll` to use the static directory ([#428](https://github.com/fuzdev/gro/pull/428))

## 0.100.3

### Patch Changes

- add icon 🌰 ([b8180ea](https://github.com/fuzdev/gro/commit/b8180ea))

## 0.100.2

### Patch Changes

- add `icon` extension property to `package_json` ([d6c22f4](https://github.com/fuzdev/gro/commit/d6c22f4))

## 0.100.1

### Patch Changes

- fix deps ([6a9a24d](https://github.com/fuzdev/gro/commit/6a9a24d))

## 0.100.0

### Minor Changes

- upgrade `@fuzdev/fuz_util@0.16` ([88ac35b](https://github.com/fuzdev/gro/commit/88ac35b))

## 0.99.0

### Minor Changes

- rename to `ProperSnakes` ([#433](https://github.com/fuzdev/gro/pull/433))

### Patch Changes

- throw on failed vite build ([#433](https://github.com/fuzdev/gro/pull/433))

## 0.98.3

### Patch Changes

- improve type of `plugin_replace` ([9165ab8](https://github.com/fuzdev/gro/commit/9165ab8))

## 0.98.2

### Patch Changes

- fix `to_package_modules` for nested modules ([a21578c](https://github.com/fuzdev/gro/commit/a21578c))

## 0.98.1

### Patch Changes

- fix `to_package_modules` ([#432](https://github.com/fuzdev/gro/pull/432))

## 0.98.0

### Minor Changes

- add `"global"` property to `package.json` to support `.well-known/package.json` instead of `"private"` ([#431](https://github.com/fuzdev/gro/pull/431))

## 0.97.0

### Minor Changes

- add `modules` to `package.json` ([#430](https://github.com/fuzdev/gro/pull/430))

## 0.96.4

### Patch Changes

- fix $env imports in deps in loader ([dc6dddc](https://github.com/fuzdev/gro/commit/dc6dddc))

## 0.96.3

### Patch Changes

- run `svelte-kit sync` before typecheck ([2794e20](https://github.com/fuzdev/gro/commit/2794e20))

## 0.96.2

### Patch Changes

- fix `gro release` to not `gro publish` if a library is not detected ([80a1dc0](https://github.com/fuzdev/gro/commit/80a1dc0))

## 0.96.1

### Patch Changes

- add default package exports to svelte files ([f0350f3](https://github.com/fuzdev/gro/commit/f0350f3))

## 0.96.0

### Minor Changes

- detect library by default by checking for @sveltejs/package ([#426](https://github.com/fuzdev/gro/pull/426))

## 0.95.8

### Patch Changes

- support absolute filenames with gen ([40a427eb](https://github.com/fuzdev/gro/commit/40a427eb))

## 0.95.7

### Patch Changes

- support json exports mapping in the loader ([07f170a](https://github.com/fuzdev/gro/commit/07f170a))

## 0.95.6

### Patch Changes

- fix loader for node_modules ([#425](https://github.com/fuzdev/gro/pull/425))
- add `--no-sync` option to `gro dev` ([#425](https://github.com/fuzdev/gro/pull/425))
- add `$lib/resolve_node_specifier.ts` ([#425](https://github.com/fuzdev/gro/pull/425))

## 0.95.5

### Patch Changes

- upgrade @fuzdev/fuz_util@0.15.1 from 0.15.0 ([52fad99](https://github.com/fuzdev/gro/commit/52fad99))

## 0.95.4

### Patch Changes

- resolve json in the loader ([#424](https://github.com/fuzdev/gro/pull/424))
- fix esbuild plugin for unknown file extensions ([#424](https://github.com/fuzdev/gro/pull/424))

## 0.95.3

### Patch Changes

- fix library detection ([c2b91bb](https://github.com/fuzdev/gro/commit/c2b91bb))

## 0.95.2

### Patch Changes

- upgrade deps ([b1264cc](https://github.com/fuzdev/gro/commit/b1264cc))

## 0.95.1

### Patch Changes

- add plugin_replace helper ([#423](https://github.com/fuzdev/gro/pull/423))
- fix loading package.json ([#423](https://github.com/fuzdev/gro/pull/423))

## 0.95.0

### Minor Changes

- rename CreateGroConfig from GroConfigCreator ([#422](https://github.com/fuzdev/gro/pull/422))
- add package.gen.ts for importing package.json data ([#419](https://github.com/fuzdev/gro/pull/419))
- change config to explictly publish `static/.well-known/package.json` and delete `gro exports` ([#419](https://github.com/fuzdev/gro/pull/419))
- rename `clean_fs.ts` from `clean.ts` ([#419](https://github.com/fuzdev/gro/pull/419))

## 0.94.2

### Patch Changes

- support prettier options to format_file ([#421](https://github.com/fuzdev/gro/pull/421))

## 0.94.1

### Patch Changes

- support typed json exports ([#420](https://github.com/fuzdev/gro/pull/420))

## 0.94.0

### Minor Changes

- fix mutation of schemas during gen ([#418](https://github.com/fuzdev/gro/pull/418))

## 0.93.1

### Patch Changes

- fix pull on deploy ([3bd77ee](https://github.com/fuzdev/gro/commit/3bd77ee))

## 0.93.0

### Minor Changes

- change `invoke_task` to require a config param ([#415](https://github.com/fuzdev/gro/pull/415))

### Patch Changes

- `git pull origin target` on deploy ([54f49ba](https://github.com/fuzdev/gro/commit/54f49ba))

## 0.92.5

### Patch Changes

- loosen package_json bugs type to accept strings ([a8cec89](https://github.com/fuzdev/gro/commit/a8cec89))

## 0.92.4

### Patch Changes

- add release task ([a088563](https://github.com/fuzdev/gro/commit/a088563))

## 0.92.3

### Patch Changes

- run git status on workspace check failure ([da4b954](https://github.com/fuzdev/gro/commit/da4b954))

## 0.92.2

### Patch Changes

- sync on `changeset` ([019d70b](https://github.com/fuzdev/gro/commit/019d70b))

## 0.92.1

### Patch Changes

- sync on `check` unless `--workspace` ([11e822e](https://github.com/fuzdev/gro/commit/11e822e))

## 0.92.0

### Minor Changes

- remove `.gro/dist` from the paths and clean task ([#413](https://github.com/fuzdev/gro/pull/413))

## 0.91.0

### Minor Changes

- upgrade to @fuzdev/fuz_util@0.15 from 0.14 ([18cbcb5](https://github.com/fuzdev/gro/commit/18cbcb5))

## 0.90.3

### Patch Changes

- invoke `gro sync` at the end of `gro upgrade` ([f0ba85c](https://github.com/fuzdev/gro/commit/f0ba85c))

## 0.90.2

### Patch Changes

- add workspace flag to `gro check` that defaults to false ([#412](https://github.com/fuzdev/gro/pull/412))

## 0.90.1

### Patch Changes

- fix `gro exports` when there is no lib directory ([#411](https://github.com/fuzdev/gro/pull/411))

## 0.90.0

### Minor Changes

- fix git helpers for CI ([#410](https://github.com/fuzdev/gro/pull/410))

## 0.89.2

### Patch Changes

- improve `gro exports` output ([c92b109](https://github.com/fuzdev/gro/commit/c92b109))
- fix publish script ([bd0d061](https://github.com/fuzdev/gro/commit/bd0d061))

## 0.89.1

### Patch Changes

- fix import.meta.env shim ([45f4734](https://github.com/fuzdev/gro/commit/45f4734))

## 0.89.0

### Minor Changes

- fix bin ([51b0a74](https://github.com/fuzdev/gro/commit/51b0a74))
- upgrade deps ([882cd1d](https://github.com/fuzdev/gro/commit/882cd1d))

## 0.88.0

### Minor Changes

- change server plugin to output to `dist_server` ([#409](https://github.com/fuzdev/gro/pull/409))

## 0.87.5

### Patch Changes

- sync on changeset ([dc5e0dd](https://github.com/fuzdev/gro/commit/dc5e0dd))
- fix server plugin paths ([9bc4685](https://github.com/fuzdev/gro/commit/9bc4685))

## 0.87.4

### Patch Changes

- relax some PackageJson types" ([284ce48](https://github.com/fuzdev/gro/commit/284ce48))

## 0.87.3

### Patch Changes

- add enum support for cli args ([3e24565](https://github.com/fuzdev/gro/commit/3e24565))

## 0.87.2

### Patch Changes

- passthrough unknown `PackageJson` properties ([f16eaab](https://github.com/fuzdev/gro/commit/f16eaab))

## 0.87.1

### Patch Changes

- add `PackageJson` schema ([#408](https://github.com/fuzdev/gro/pull/408))

## 0.87.0

### Minor Changes

- remove execSync usage ([#407](https://github.com/fuzdev/gro/pull/407))
- upgrade to `@fuzdev/fuz_util@0.14.0` from `0.13.0` ([#407](https://github.com/fuzdev/gro/pull/407))

## 0.86.0

### Minor Changes

- add `package.json` to the static `/.well-known` directory by default ([#405](https://github.com/fuzdev/gro/pull/405))
- fix `gro deploy` ([#406](https://github.com/fuzdev/gro/pull/406))
- add git utils ([#406](https://github.com/fuzdev/gro/pull/406))
- add `package_json` to the config for customizing exports and `.well-known` ([#405](https://github.com/fuzdev/gro/pull/405))

### Patch Changes

- fix `gro deploy --reset` flag ([#389](https://github.com/fuzdev/gro/pull/389))

## 0.85.0

### Minor Changes

- rename some dir constants to prefix with GRO* instead of BUILD* ([#404](https://github.com/fuzdev/gro/pull/404))
- merge adapt into plugins ([#404](https://github.com/fuzdev/gro/pull/404))

### Patch Changes

- fill out the package json type ([#403](https://github.com/fuzdev/gro/pull/403))

## 0.84.1

### Patch Changes

- fix types in fixtures ([#402](https://github.com/fuzdev/gro/pull/402))

## 0.84.0

### Minor Changes

- rename to `@fuzdev/gro` from `@feltjs/gro` ([#400](https://github.com/fuzdev/gro/pull/400))
- flatten directories ([#400](https://github.com/fuzdev/gro/pull/400))

## 0.83.0

### Minor Changes

- replace `@feltjs/util` with `@fuzdev/fuz_util` ([8fcac65](https://github.com/fuzdev/gro/commit/8fcac65))

## 0.82.9

### Patch Changes

- infer the `gro changeset` access arg from `package.json#private` ([71ea36f](https://github.com/fuzdev/gro/commit/71ea36f))

## 0.82.8

### Patch Changes

- always init config ([5262967](https://github.com/fuzdev/gro/commit/5262967))

## 0.82.7

### Patch Changes

- add `gro changeset` task ([#399](https://github.com/fuzdev/gro/pull/399))

## 0.82.6

### Patch Changes

- make `@sveltejs/package` optional and warn when adapting the library ([989c39c](https://github.com/fuzdev/gro/commit/989c39c))

## 0.82.5

### Patch Changes

- import uvu dynamically to make it optional ([#398](https://github.com/fuzdev/gro/pull/398))

## 0.82.4

### Patch Changes

- upgrade es-module-lexer ([#397](https://github.com/fuzdev/gro/pull/397))

## 0.82.3

### Patch Changes

- remove config caching ([693e4ceb](https://github.com/fuzdev/gro/commit/693e4ceb))

## 0.82.2

### Patch Changes

- improve `gro exports` ([#396](https://github.com/fuzdev/gro/pull/396)) ([#396](https://github.com/fuzdev/gro/pull/396))

## 0.82.1

### Patch Changes

- fix deploy task ([#395](https://github.com/fuzdev/gro/pull/395))

## 0.82.0

### Minor Changes

- replace the build system with a loader and esbuild plugins and integrate svelte-package ([#382](https://github.com/fuzdev/gro/pull/382))
- rename VocabSchema to JsonSchema ([#394](https://github.com/fuzdev/gro/pull/394))

## 0.81.2

### Patch Changes

- prefer project-local changeset command if available ([#393](https://github.com/fuzdev/gro/pull/393))

## 0.81.1

### Patch Changes

- detect changeset version failure ([edf3214](https://github.com/fuzdev/gro/commit/edf3214))

## 0.81.0

### Minor Changes

- change gro publish to use changesets ([#385](https://github.com/fuzdev/gro/pull/385))

## 0.80.0

- **break**: replace `cheap-watch` with `chokidar` and `fast-glob`
  ([#386](https://github.com/fuzdev/gro/pull/386))
- fix `gen` error on startup for projects with no gen modules
  ([#387](https://github.com/fuzdev/gro/pull/387))

## 0.79.1

- add `gro commit` task
  ([#379](https://github.com/fuzdev/gro/pull/379))
- add `gen` plugin that defaults to watch mode
  ([#283](https://github.com/fuzdev/gro/pull/283),
  [#381](https://github.com/fuzdev/gro/pull/381))
- add `--reset` and `--origin` flags to `gro deploy`
  ([#383](https://github.com/fuzdev/gro/pull/383))

## 0.79.0

- **break**: upgrade to `@fuzdev/fuz_util@0.9.0` and peer dep `svelte@4`,
  and change peer deps to avoid breaking changes
  ([#378](https://github.com/fuzdev/gro/pull/378))

## 0.78.2

- upgrade deps
  ([#377](https://github.com/fuzdev/gro/pull/377))

## 0.78.1

- upgrade deps
  ([#376](https://github.com/fuzdev/gro/pull/376))

## 0.78.0

- **break**: change schema bundling to use the parsed name in `$defs` instead of the `$id`
  ([commit](https://github.com/fuzdev/gro/commit/b5623e2867196823bb146c4794dde035ce6e7fc5))

## 0.77.0

- **break**: move `toJson_Schema` to `$lib/schemaHelpers.ts` and export all of `$lib/schema.ts`
  ([commit](https://github.com/fuzdev/gro/commit/21a107633f950ffb540b180b57fc3227146993c5))

## 0.76.1

- upgrade deps
  ([commit](https://github.com/fuzdev/gro/commit/538bff4b8f092c0e176fc242cab9da7bfa3f2db9))

## 0.76.0

- **break**: change `toJson_Schema` to not suffix with `.json`, and add the `bundle_schemas` helper
  ([#372](https://github.com/fuzdev/gro/pull/372))

## 0.75.5

- upgrade `@ryanatkn/json-schema-to-typescript` dep,
  changing `tsImport` type from `string` to `string | Array<string>`
  ([commit](https://github.com/fuzdev/gro/commit/ff088ae789486fb1348d894078c3a1c2bbb2974a))

## 0.75.4

- export `JsonSchema` type from root
  ([commit](https://github.com/fuzdev/gro/commit/8b3956994060165cccf7c0d6b692ea8e89b7e63a))

## 0.75.3

- upgrade deps
  ([#371](https://github.com/fuzdev/gro/pull/371))

## 0.75.2

- de-dupe when generating types
  ([commit](https://github.com/fuzdev/gro/commit/01d02a17a5b050b63d7699371e559802fed6407d))

## 0.75.1

- export `to_gen_context_imports` from `lib/run_gen`
  ([commit](https://github.com/fuzdev/gro/commit/980f75bf43f77cb3107b446c079cc401c88b3f94))

## 0.75.0

- **break**: change schema type generation to infer `tsType` and `tsImport` from `$ref` and `instanceof`
  ([#370](https://github.com/fuzdev/gro/pull/370))
- **break**: bump peer dep `svelte@0.58.0`
  ([#370](https://github.com/fuzdev/gro/pull/370))

## 0.74.0

- **break**: fix schema type generation to be shallow, excluding the types of referenced schemas
  ([#367](https://github.com/fuzdev/gro/pull/367))

## 0.73.0

- **break**: upgrade dep `@fuzdev/fuz_util@0.8.0` and `esbuild@0.17.0`
  ([#366](https://github.com/fuzdev/gro/pull/366))
- `npm i` before `gro build`, add `--no-install` flag to override
  ([#365](https://github.com/fuzdev/gro/pull/365))

## 0.72.0

- **break**: upgrade dep `@fuzdev/fuz_util@0.7.4`
  ([commit](https://github.com/fuzdev/gro/commit/731007d1533f981e524e318c00af3a6fa861e909))

## 0.71.0

- **break**: upgrade dep `@fuzdev/fuz_util@0.6.0`
  ([#364](https://github.com/fuzdev/gro/pull/364))

## 0.70.5

- quieter logging

## 0.70.4

- bump package-lock version to 3after npm upgrade

## 0.70.3

- upgrade `@fuzdev/fuz_util`
  ([#362](https://github.com/fuzdev/gro/pull/362))

## 0.70.2

- fix production types for builds that don't specify a value
  ([#363](https://github.com/fuzdev/gro/pull/363))
- undo adding TypeScript as a peer dependency

## 0.70.1

- add TypeScript as a peer dependency

## 0.70.0

- **break**: remove `tsconfig` arg from `gro typecheck` since `svelte-check` 3 no longer needs it
  ([#360](https://github.com/fuzdev/gro/pull/360))
- **break**: move the SvelteKit build instead of copying it
  ([5719273](https://github.com/fuzdev/gro/commit/5719273aba7b3634fdd6c4f8a6d4714a1512b8d4))

## 0.69.0

- **break**: upgrade deps including SvelteKit 1.0
  ([#359](https://github.com/fuzdev/gro/pull/359))

## 0.68.2

- relax the type of `RawGenResult` to let the array have any value
  ([6faf95b](https://github.com/fuzdev/gro/commit/6faf95b36bc0769aee45d0c96454a445ebb8485c))

## 0.68.1

- relax the type of `RawGenResult` to include an ignored `null`
  ([a8cf511](https://github.com/fuzdev/gro/commit/a8cf51129ff53a4d8257c1f2e728aee2ffd7f2ac))

## 0.68.0

- **break**: move `types` from the main config to the build config
  ([#358](https://github.com/fuzdev/gro/pull/358))

## 0.67.3

- add `svelte` property of published `package.json`
- upgrade deps

## 0.67.2

- remove `readonly` from `BuildConfigInput[]` usage

## 0.67.1

- fix `$app/environment` `dev` import

## 0.67.0

- upgrade `@fuzdev/fuz_util@0.3.0`

## 0.66.1

- fix imports

## 0.66.0

- **break**: upgrade `@fuzdev/fuz_util`
  ([#356](https://github.com/fuzdev/gro/pull/356))

## 0.65.0

- **break**: depend on `@fuzdev/fuz_util` instead of `@feltjs/felt-ui`
  ([#355](https://github.com/fuzdev/gro/pull/355))

## 0.64.0

- **break**: upgrade peer dep `@feltjs/felt-ui@0.42.0`
  ([#354](https://github.com/fuzdev/gro/pull/354))

## 0.63.2

- call `svelte-kit sync` only if it's installed
  ([#353](https://github.com/fuzdev/gro/pull/353))

## 0.63.1

- pass `--skipLibCheck` to `tsc` on `gro build` so it can be disabled in dependents' tsconfigs
  ([#352](https://github.com/fuzdev/gro/pull/352))
- always run `svelte-kit sync` instead of checking for the directory first
  ([#352](https://github.com/fuzdev/gro/pull/352))

## 0.63.0

- **break**: change `gro.config.ts` to export a default value instead of a `config` property
  ([#348](https://github.com/fuzdev/gro/pull/348))
- **break**: change `plugin/gro_plugin_server.ts` to a no-op outside of dev mode
  ([#347](https://github.com/fuzdev/gro/pull/347))
- fix `gro deploy` for unfetched branches
  ([#350](https://github.com/fuzdev/gro/pull/350))
- upgrade deps
  ([#351](https://github.com/fuzdev/gro/pull/351))

## 0.62.4

- skip `gro test` if `uvu` is not installed, and log a warning
  ([#346](https://github.com/fuzdev/gro/pull/346))

## 0.62.3

- do not run `tsc` in `gro check` if `svelte-check` is available
  ([#345](https://github.com/fuzdev/gro/pull/345))

## 0.62.2

- fix task arg definitions

## 0.62.1

- upgrade deps
  ([#342](https://github.com/fuzdev/gro/pull/342))
- use [zod](https://github.com/colinhacks/zod) for task schemas
  ([#344](https://github.com/fuzdev/gro/pull/344))

## 0.62.0

- **break**: upgrade deps
  ([#341](https://github.com/fuzdev/gro/pull/341))
- respect the `types` config property for Svelte files
  ([#341](https://github.com/fuzdev/gro/pull/341))

## 0.61.3

- fix type parsing for Svelte files
  ([#340](https://github.com/fuzdev/gro/pull/340))

## 0.61.2

- change `gro typecheck` to always run both `tsc` and `svelte-check`
  instead of stopping when `tsc` finds an error
  ([#339](https://github.com/fuzdev/gro/pull/339))

## 0.61.1

- upgrade SvelteKit import mocks
  ([#338](https://github.com/fuzdev/gro/pull/338))

## 0.61.0

- **break**: upgrade deps and remove peer deps for `@sveltejs/kit`, `uvu`, and `typescript`
  ([#337](https://github.com/fuzdev/gro/pull/337))

## 0.60.2

- fix `gro deploy` when the deployment branch isn't loaded locally
  ([#336](https://github.com/fuzdev/gro/pull/336))

## 0.60.1

- add exclude functionality to the upgrade task: `gro upgrade foo bar`
  ([#335](https://github.com/fuzdev/gro/pull/335))

## 0.60.0

- **break**: upgrade deps
  ([#334](https://github.com/fuzdev/gro/pull/334))
- add upgrade task
  ([#334](https://github.com/fuzdev/gro/pull/334))

## 0.59.0

- **break**: upgrade deps
  ([#332](https://github.com/fuzdev/gro/pull/332))

## 0.58.0

- **break**: upgrade deps
  ([#331](https://github.com/fuzdev/gro/pull/331))

## 0.57.0

- **break**: upgrade deps
  ([#330](https://github.com/fuzdev/gro/pull/330))

## 0.56.0

- **break**: rename adapter and plugin modules to use dash-case
  ([#327](https://github.com/fuzdev/gro/pull/327))
- upgrade some deps
  ([#326](https://github.com/fuzdev/gro/pull/326))

## 0.55.2

- build only the included TypeScript files to `dist/src/`
  ([#325](https://github.com/fuzdev/gro/pull/325))

## 0.55.1

- add `format` option to gen files, defaults to `true`
  ([#324](https://github.com/fuzdev/gro/pull/324))

## 0.55.0

- **break**: remove the dev server, including `gro serve` and project metadata
  ([#321](https://github.com/fuzdev/gro/pull/321))
- always build before running tasks
  ([#322](https://github.com/fuzdev/gro/pull/322))

## 0.54.0

- **break**: upgrade deps including peer dep for `@feltjs/felt-ui@0.26.0`
  ([#320](https://github.com/fuzdev/gro/pull/320))
- **break**: remove `deploymentMode` option from SvelteKit app adapter
  ([#320](https://github.com/fuzdev/gro/pull/320))

## 0.53.0

- **break**: upgrade `@sveltejs/kit@1.0.0-next.298` and run `svelte-kit sync` before typechecking
  ([#317](https://github.com/fuzdev/gro/pull/317))

## 0.52.3

- register schemas for gen and search for them before the file resolver
  ([#316](https://github.com/fuzdev/gro/pull/316))

## 0.52.2

- improve safety of `gro deploy` and update its docs
  ([#315](https://github.com/fuzdev/gro/pull/315))

## 0.52.1

- fix args type for `invoke_task`
  ([#314](https://github.com/fuzdev/gro/pull/314))

## 0.52.0

- **break**: validate task `args` from schemas,
  setting defaults automatically and disallowing additional properties
  ([#313](https://github.com/fuzdev/gro/pull/313))
- **break**: change `invoke_task` to no longer forward `args` automatically
  ([#313](https://github.com/fuzdev/gro/pull/313))
- **break**: add generic agnostic command args forwarding
  [using the `--` pattern](src/docs/task.md#task-args-forwarding)
  ([#313](https://github.com/fuzdev/gro/pull/313))
- **break**: remove `-v` alias for `gro --version`
  ([#313](https://github.com/fuzdev/gro/pull/313))

## 0.51.3

- fix task name printing for task directories
  ([#312](https://github.com/fuzdev/gro/pull/312))

## 0.51.2

- support task dirs
  ([#311](https://github.com/fuzdev/gro/pull/311))

## 0.51.1

- fix value printing in args logging
  ([#310](https://github.com/fuzdev/gro/pull/310))

## 0.51.0

- **break**: rename the `gro publish` `branch` arg to `source` and add `target` branch arg
  ([#306](https://github.com/fuzdev/gro/pull/306))
- **break**: move `mapBundleOptions` from args to the Node library adapter options
  ([#306](https://github.com/fuzdev/gro/pull/306))
- add schema information to task args
  ([#306](https://github.com/fuzdev/gro/pull/306))
- add `--help` flag to `gro` and `gro taskname`, along with `gro help` alias
  ([#306](https://github.com/fuzdev/gro/pull/306), [#307](https://github.com/fuzdev/gro/pull/307))
- combine imports in generated schema types
  ([#304](https://github.com/fuzdev/gro/pull/304))
- add CLI opt-outs to `gro check` for `no-typecheck`, `no-test`, `no-gen`, `no-format`, & `no-lint`
  ([#305](https://github.com/fuzdev/gro/pull/305))
- fix inline type parsing to include type dependencies
  ([#308](https://github.com/fuzdev/gro/pull/308))

## 0.50.5

- make `gro check` fail on lint warnings
  ([#302](https://github.com/fuzdev/gro/pull/302))
- pass through args in `gro lint` to `eslint`
  ([#302](https://github.com/fuzdev/gro/pull/302))

## 0.50.4

- run `eslint` in `gro check` if it's available
  ([#301](https://github.com/fuzdev/gro/pull/301))

## 0.50.3

- fix default config to build types only for production
  ([#300](https://github.com/fuzdev/gro/pull/300))

## 0.50.2

- add `.schema.` files to system build
  ([#299](https://github.com/fuzdev/gro/pull/299))

## 0.50.1

- add `gro gen` support for `.schema.` files with automatically generated types
  ([#298](https://github.com/fuzdev/gro/pull/298))

## 0.50.0

- **break**: upgrade `@feltjs/felt-ui@0.18.0`
  ([#297](https://github.com/fuzdev/gro/pull/297))

## 0.49.0

- **break**: remove `svelte-check` as a dependency,
  but still call it in `gro typecheck` if installed
  ([#296](https://github.com/fuzdev/gro/pull/296))

## 0.48.0

- **break**: remove Rollup and its plugins as a dependency, and use esbuild for bundling instead;
  this changes the Node library adapter interface to use `mapBundleOptions`
  and removes the Rollup mapping functions
  ([#295](https://github.com/fuzdev/gro/pull/295))

## 0.47.5

- mock SvelteKit imports like `'$app/navigation'` for tests
  ([#294](https://github.com/fuzdev/gro/pull/294))

## 0.47.4

- fix 404 page for GitHub pages with SvelteKit adapter
  ([#293](https://github.com/fuzdev/gro/pull/293))

## 0.47.3

- make the `throttle` cache key optional
  ([#290](https://github.com/fuzdev/gro/pull/290))
- attempt to fix deps broken by [#286](https://github.com/fuzdev/gro/pull/286)
  ([#291](https://github.com/fuzdev/gro/pull/291))

## 0.47.2

- fix bug with the `throttle` `delay` option
  ([#289](https://github.com/fuzdev/gro/pull/289))

## 0.47.1

- fix `svelte-check` to scope to `src/`
  ([#273](https://github.com/fuzdev/gro/pull/273))
- fix writing source meta concurrently
  ([#288](https://github.com/fuzdev/gro/pull/288))
- add `util/throttle.ts`
  ([#288](https://github.com/fuzdev/gro/pull/288))

## 0.47.0

- **break**: update deps
  ([#286](https://github.com/fuzdev/gro/pull/286))

## 0.46.0

- **break**: change task `dev` property to `production`
  ([#284](https://github.com/fuzdev/gro/pull/284))
- fix `process.env.NODE_ENV` for production tasks
  ([#284](https://github.com/fuzdev/gro/pull/284),
  [#287](https://github.com/fuzdev/gro/pull/287))

## 0.45.2

- clean dist for production builds
  ([#285](https://github.com/fuzdev/gro/pull/285))

## 0.45.1

- fix peer dependency versions
  ([#282](https://github.com/fuzdev/gro/pull/282))

## 0.45.0

- **break**: remove frontend build support in favor of SvelteKit
  ([#281](https://github.com/fuzdev/gro/pull/281))

## 0.44.3

- process `.js` files with the esbuild builder
  ([#280](https://github.com/fuzdev/gro/pull/280))

## 0.44.2

- add optional `paths` to `Filer` and improve its tests
  ([#276](https://github.com/fuzdev/gro/pull/276))
- make `gro test` ignore sourcemap files by default so patterns don't need `.js$`
  ([#277](https://github.com/fuzdev/gro/pull/277))
- make `gro gen` build before running
  ([#279](https://github.com/fuzdev/gro/pull/279))

## 0.44.1

- change default config to not run the api server during build
  ([#275](https://github.com/fuzdev/gro/pull/275))

## 0.44.0

- **break**: upgrade `@feltjs/felt-ui@0.13.0` and make it a peer dependency
  ([#274](https://github.com/fuzdev/gro/pull/274))

## 0.43.0

- **break**: upgrade `@feltjs/felt-ui@0.12.0` and `rollup@2.57.0`
  ([#272](https://github.com/fuzdev/gro/pull/272))

## 0.42.0

- **break**: add `svelte-check` and typecheck Svelte files in `gro typecheck` and `gro check`
  ([#271](https://github.com/fuzdev/gro/pull/271))

## 0.41.1

- support js imports
  ([#270](https://github.com/fuzdev/gro/pull/270))

## 0.41.0

- **break**: update deps and remove most peer deps
  ([#269](https://github.com/fuzdev/gro/pull/269))
- make `gro deploy` safer by excluding branches `'main'` and `'master'` unless `--force`
  ([#268](https://github.com/fuzdev/gro/pull/268))

## 0.40.0

- rename to `camelCase` from `snake_case`
  ([#267](https://github.com/fuzdev/gro/pull/267))

## 0.39.1

- fix dev server plugin defaults to not load for prod builds
  ([#265](https://github.com/fuzdev/gro/pull/265))

## 0.39.0

- **break**: rename to `PascalCase` from `ProperSnakeCase`
  ([#263](https://github.com/fuzdev/gro/pull/263),
  [#264](https://github.com/fuzdev/gro/pull/264))

## 0.38.0

- **break**: require Node >=16.6.0
  ([#262](https://github.com/fuzdev/gro/pull/262))

## 0.37.0

- **break**: remove `main_test` config option and behavior
  ([#261](https://github.com/fuzdev/gro/pull/261))

## 0.36.1

- add some convenience peer dependencies like `@types/source-map-support` --
  is this a good idea?
  ([#260](https://github.com/fuzdev/gro/pull/260))

## 0.36.0

- **break**: fix test sourcemaps by adding
  [`GroConfig` option `main_test`](src/docs/config.md#main_test),
  which initializes projects with a conventional `lib/main.test.ts`
  for installing sourcemaps and other global test concerns (update: reverted in 0.37.0)
  ([#259](https://github.com/fuzdev/gro/pull/259))
- add some peer deps
  ([#259](https://github.com/fuzdev/gro/pull/259))

## 0.35.0

- **break**: remove dev task event `dev.create_server` and add `dev.create_context`
  ([#258](https://github.com/fuzdev/gro/pull/258))
- make the dev server a plugin
  ([#258](https://github.com/fuzdev/gro/pull/258))

## 0.34.3

- clean during `gro build` except when invoked by `gro publish` and `gro deploy`
  ([#255](https://github.com/fuzdev/gro/pull/255))

## 0.34.2

- forward args to `svelte-kit dev` and `svelte-kit build`
  ([#254](https://github.com/fuzdev/gro/pull/254))
- improve type matcher regexp
  ([#253](https://github.com/fuzdev/gro/pull/253))

## 0.34.1

- fix type publishing
  ([#252](https://github.com/fuzdev/gro/pull/252))

## 0.34.0

- **break**: remove `cjs` and `esm` options from `gro_adapter_library`;
  for now only ESM is supported, to be revisited when we support package exports;
  in the meantime users can fork the adapter for commonjs outputs
  ([#251](https://github.com/fuzdev/gro/pull/251))
- **break**: disallow running some tasks in development: `gro build`, `gro deploy`, `gro publish`
  ([#251](https://github.com/fuzdev/gro/pull/251))
- rename `src/lib/adapt.ts` module and improve docs for `adapt` and `plugin`
  ([#250](https://github.com/fuzdev/gro/pull/250))

## 0.33.1

- fix `gro_adapter_gro_frontend` by prebundling externals only in development
  ([#249](https://github.com/fuzdev/gro/pull/249))
- upgrade `es-module-lexer@0.7.1`
  ([#248](https://github.com/fuzdev/gro/pull/248))

## 0.33.0

- **break**: refactor `postprocess` into builders and delete `Build` and `BuildResult`
  ([#243](https://github.com/fuzdev/gro/pull/243))
- **break**: rename builders and builder util
  ([#244](https://github.com/fuzdev/gro/pull/244))
- **break**: merge `util/path_filter.ts` and `util/file.ts` into `util/filter.ts`,
  rename `IdFilter` from `FileFilter`, and add `IdStatsFilter`
  ([#246](https://github.com/fuzdev/gro/pull/246))
- support JSON imports
  ([#245](https://github.com/fuzdev/gro/pull/245))
- ignore unwanted assets in frontend build
  ([#242](https://github.com/fuzdev/gro/pull/242),
  [#246](https://github.com/fuzdev/gro/pull/246))

## 0.32.1

- change `gro deploy` to infer default `dirname`
  ([#241](https://github.com/fuzdev/gro/pull/241))

## 0.32.0

- **break**: add `gro_adapter_gro_frontend` adapter
  ([#231](https://github.com/fuzdev/gro/pull/231))

## 0.31.0

- **break**: change `gro_adapter_library` option `type` to boolean `bundle`
  ([#239](https://github.com/fuzdev/gro/pull/239))
- support outputting Svelte source files in production builds
  ([#239](https://github.com/fuzdev/gro/pull/239))

## 0.30.3

- fix dynamic import parsing to allow non-interpolated template string literals
  ([#238](https://github.com/fuzdev/gro/pull/238))

## 0.30.2

- fix import parsing to ignore non-literal dynamic imports
  ([#237](https://github.com/fuzdev/gro/pull/237))

## 0.30.1

- fix conversion of absolute specifiers to include `./` if bare
  ([#236](https://github.com/fuzdev/gro/pull/236))
- upgrade to esbuild@0.12.15
  ([#235](https://github.com/fuzdev/gro/pull/235))

## 0.30.0

- **break**: change default server to `src/lib/server/server.ts` from `src/server/server.ts`
  ([#234](https://github.com/fuzdev/gro/pull/234))

## 0.29.0

- **break**: rename plugins, adapters, and builders to `snake_case` from `kebab-case`
  ([#232](https://github.com/fuzdev/gro/pull/232))
- update deps
  ([#233](https://github.com/fuzdev/gro/pull/233))

## 0.28.3

- add Gro frontend support with `gro_adapter_gro_frontend`
  ([#231](https://github.com/fuzdev/gro/pull/231))
- fix buildless builds to handle SvelteKit-only frontend projects
  ([#230](https://github.com/fuzdev/gro/pull/230))

## 0.28.2

- upgrade to `@feltjs/felt-ui@0.3.0`
  ([#229](https://github.com/fuzdev/gro/pull/229))

## 0.28.1

- fix library adapter `package.json` paths
  ([#228](https://github.com/fuzdev/gro/pull/228))

## 0.28.0

- **break**: remove `gro start` task
  ([#223](https://github.com/fuzdev/gro/pull/223))
- add `Plugin`s and `config.plugin` to support SvelteKit and Node servers
  ([#223](https://github.com/fuzdev/gro/pull/223),
  [#224](https://github.com/fuzdev/gro/pull/224))
- implement Node server config and adapter
  ([#223](https://github.com/fuzdev/gro/pull/223))
- implement library adapter
  ([#226](https://github.com/fuzdev/gro/pull/226),
  [#227](https://github.com/fuzdev/gro/pull/227))

## 0.27.3

- fix default library build to include types in production
  ([#222](https://github.com/fuzdev/gro/pull/222))

## 0.27.2

- fix deploy task
  ([#221](https://github.com/fuzdev/gro/pull/221))

## 0.27.1

- support absolute path imports `$lib/` and `src/`
  ([#153](https://github.com/fuzdev/gro/pull/153))
- infer `npm link` in Node library adapter
  ([#218](https://github.com/fuzdev/gro/pull/218))
- fix build config validation and normalization
  ([#220](https://github.com/fuzdev/gro/pull/220))

## 0.27.0

- **break**: convert to `snake_case` from `camelCase`
  ([#210](https://github.com/fuzdev/gro/pull/210))
- **break**: rename `'system'` build from `'node'`
  ([#207](https://github.com/fuzdev/gro/pull/207))
- **break**: add `'config'` build to simplify internals
  ([#207](https://github.com/fuzdev/gro/pull/207),
  [#212](https://github.com/fuzdev/gro/pull/212))
- **break**: build configs now fail to validate if any input path strings do not exist
  ([#207](https://github.com/fuzdev/gro/pull/207))
- **break**: rename `load_config` from `loadGroConfig`
  ([#207](https://github.com/fuzdev/gro/pull/207))
- **break**: change `validate_build_configs` function signature
  ([#207](https://github.com/fuzdev/gro/pull/207))
- **break**: change `gro_adapter_sveltekit_app` output so it composes with others
  ([#207](https://github.com/fuzdev/gro/pull/207))
- **break**: rename the `Task` `summary` property from `description`
  ([#212](https://github.com/fuzdev/gro/pull/212))
- **break**: rename `content` from `contents`
  ([#216](https://github.com/fuzdev/gro/pull/216))
- add `no-watch` arg to `gro dev`
  ([#211](https://github.com/fuzdev/gro/pull/211))
- rename some args in `gro dev` and `gro serve`
  ([#211](https://github.com/fuzdev/gro/pull/211))
- optimize writing source meta to disk
  ([#214](https://github.com/fuzdev/gro/pull/214))
- add `typemap` option to Gro config
  ([#215](https://github.com/fuzdev/gro/pull/215))
- add `types` option to Gro config
  ([#215](https://github.com/fuzdev/gro/pull/215))

## 0.26.2

- support publishing library builds
  ([#209](https://github.com/fuzdev/gro/pull/209))
- fix typemaps for production builds
  ([#209](https://github.com/fuzdev/gro/pull/209))

## 0.26.1

- rename default `'library'` build from `'lib'`
  ([#208](https://github.com/fuzdev/gro/pull/208))

## 0.26.0

- **break**: move util to `@feltjs/felt-ui` and add it as a dependency
  ([#206](https://github.com/fuzdev/gro/pull/206))

## 0.25.2

- properly detect unclean git state for `gro deploy`
  ([#205](https://github.com/fuzdev/gro/pull/205))

## 0.25.1

- add `.nojekyll` support to frontend adapters for GitHub Pages compatibility
  ([#204](https://github.com/fuzdev/gro/pull/204))

## 0.25.0

- **break**: upgrade to latest SvelteKit, changing the dir `.svelte` to `.svelte-kit`
  ([#202](https://github.com/fuzdev/gro/pull/202))
- add SvelteKit app adapter
  ([#193](https://github.com/fuzdev/gro/pull/193))
- fix `gro deploy`
  ([#193](https://github.com/fuzdev/gro/pull/193))

## 0.24.1

- separate source meta for dev and prod
  ([#201](https://github.com/fuzdev/gro/pull/201))

## 0.24.0

- **break**: extend config with detected defaults
  ([#199](https://github.com/fuzdev/gro/pull/199))
- improve `gro publish` errors and suport restricted packages
  ([#199](https://github.com/fuzdev/gro/pull/199))

## 0.23.7

- fix `gro publish` arg forwarding to `npm version`
  ([#200](https://github.com/fuzdev/gro/pull/200))
- fix `gro publish` to initialize scoped packages
  ([#200](https://github.com/fuzdev/gro/pull/200))

## 0.23.6

- tweak publish steps to handle failure better

## 0.23.5

- tweak publishing errors

## 0.23.4

- fix typemap source path
  ([#198](https://github.com/fuzdev/gro/pull/198))
- improve publishing changelog UX
  ([#197](https://github.com/fuzdev/gro/pull/197))

## 0.23.3

- generate types for bundled production builds
  ([#196](https://github.com/fuzdev/gro/pull/196))

## 0.23.2

- generate types for production builds
  ([#194](https://github.com/fuzdev/gro/pull/194),
  [#195](https://github.com/fuzdev/gro/pull/195))

## 0.23.1

- fix arg forwarding
  ([#191](https://github.com/fuzdev/gro/pull/191),
  [#192](https://github.com/fuzdev/gro/pull/192))
- fix `gro test` in production, also fixing `gro publish`
  ([#192](https://github.com/fuzdev/gro/pull/192))

## 0.23.0

- **break**: rename `gro publish` from `gro version` and improve its safety
  ([#189](https://github.com/fuzdev/gro/pull/189))
- make `config.builds` optional and accept non-array values
  ([#189](https://github.com/fuzdev/gro/pull/189))
- document more about `adapt`
  ([#189](https://github.com/fuzdev/gro/pull/189))

## 0.22.0

- **break**: redesign `gro publish` and `gro deploy`
  ([#187](https://github.com/fuzdev/gro/pull/187))
- **break**: add [`Adapter` system](/src/docs/adapt.md) and
  [Node library adapter](/src/lib/gro_adapter_library.ts)
  ([#187](https://github.com/fuzdev/gro/pull/187))
- **break**: add a default `"node"` build named `"node"` if one is not defined
  ([#187](https://github.com/fuzdev/gro/pull/187))
- **break**: rename `toObtainable` from `createObtainable`
  ([#187](https://github.com/fuzdev/gro/pull/187))
- **break**: rename `Filesystem.write_file` from `outputFile`
  ([#188](https://github.com/fuzdev/gro/pull/188))
- **break**: upgrade to `fs-extra@10.0.0`
  ([#188](https://github.com/fuzdev/gro/pull/188))

## 0.21.6

- hack: add temporary support for extensionless import paths to internal modules
  ([#186](https://github.com/fuzdev/gro/pull/186))

## 0.21.5

- export config types and helpers from root
  ([#185](https://github.com/fuzdev/gro/pull/185))

## 0.21.4

- export `UnreachableError` and time util from root
  ([#184](https://github.com/fuzdev/gro/pull/184))

## 0.21.3

- force push on `gro deploy`
  ([#183](https://github.com/fuzdev/gro/pull/183))

## 0.21.2

- ignore error on failed `git pull` during `gro deploy`
  ([#182](https://github.com/fuzdev/gro/pull/182))

## 0.21.1

- change `gro gen` to handle failed formatting gracefully
  ([#181](https://github.com/fuzdev/gro/pull/181))

## 0.21.0

- **break**: change `gro clean` args to longhand and
  add option `--git` to prune dead branches
  ([#180](https://github.com/fuzdev/gro/pull/180))
- fix `gro build` to copy files to `dist/` with `src/lib/build/dist.ts` helper `copy_dist`
  ([#179](https://github.com/fuzdev/gro/pull/179))

## 0.20.4

- fix `gro deploy` to first pull the remote deploy branch
  ([#178](https://github.com/fuzdev/gro/pull/178))

## 0.20.3

- fix production builds
  ([#177](https://github.com/fuzdev/gro/pull/177))

## 0.20.2

- refactor Gro's production build
  ([#176](https://github.com/fuzdev/gro/pull/176))

## 0.20.1

- upgrade to npm 7 and its v2 lockfile
  ([#174](https://github.com/fuzdev/gro/pull/174),
  [#175](https://github.com/fuzdev/gro/pull/175))

## 0.20.0

- **break**: rework the `Filesystem` interfaces
  ([#173](https://github.com/fuzdev/gro/pull/173))
  - export a `fs` instance from each `Filesystem` implementation
    and do not export individual functions
  - rename `pathExists` to `exists`
  - remove `readJson`
- add abstract class `Fs` and implement `MemoryFs`
  to complement the `fs-extra` implementation at `src/lib/node.ts`
  ([#173](https://github.com/fuzdev/gro/pull/173))

## 0.19.0

- **break**: extract the `Filesystem` interface and
  thread it everywhere from `src/lib/invoke.ts` and tests
  ([#171](https://github.com/fuzdev/gro/pull/171))
- **break**: replace `src/lib/gitignore.ts` helper `is_gitignored`
  with `src/lib/path_filter.ts` helper `to_path_filter`
  ([#172](https://github.com/fuzdev/gro/pull/172))

## 0.18.2

- fix `gro start` task to serve static builds to port 3000 like the others
  ([#170](https://github.com/fuzdev/gro/pull/170))

## 0.18.1

- fix `gro start` task to work with SvelteKit and the Node server if detected
  ([#169](https://github.com/fuzdev/gro/pull/169))

## 0.18.0

- **break**: change the interface of `gro dev` and `gro build` to support the Node server
  ([#168](https://github.com/fuzdev/gro/pull/168))

## 0.17.1

- fix `gro format` to whitelist files off root when formatting `src/`
  ([#166](https://github.com/fuzdev/gro/pull/166))
- fix `gro test` to gracefully handle projects with no Gro build outputs
  ([#167](https://github.com/fuzdev/gro/pull/167))
- run `gro check` before building in `gro version`
  ([#167](https://github.com/fuzdev/gro/pull/167))

## 0.17.0

- **break**: rename `src/lib/equal.ts` module from `deepEqual.ts`
  ([#162](https://github.com/fuzdev/gro/pull/162))
- **break**: rename `GroConfigPartial` from `PartialGroConfig`
  and `BuildConfigPartial` from `PartialBuild_Config`
  ([#164](https://github.com/fuzdev/gro/pull/164))
- make serve task work for production SvelteKit builds
  ([#163](https://github.com/fuzdev/gro/pull/163))
- add `src/lib/gitignore.ts` with `is_gitignored` and `load_gitignore_filter`
  ([#165](https://github.com/fuzdev/gro/pull/165))
- add helper `to_sveltekit_base_path` to `src/lib/build/sveltekit_helpers.ts`
  ([#163](https://github.com/fuzdev/gro/pull/163))
- default to some environment variables
  for undefined Gro config properties:
  `GRO_HOST`, `GRO_PORT`, `GRO_LOG_LEVEL`
  ([#162](https://github.com/fuzdev/gro/pull/162))
- export the logger API from root and fix it
  ([#162](https://github.com/fuzdev/gro/pull/162))

## 0.16.0

- **break**: rename some paths constants for consistency
  ([#161](https://github.com/fuzdev/gro/pull/161))
- patch SvelteKit bug with duplicate build directories
  ([#160](https://github.com/fuzdev/gro/pull/160))
- add [contributing.md🌄](./contributing.md)
  ([#108](https://github.com/fuzdev/gro/pull/108))

## 0.15.0

- **break**: make `src/lib/build.task.ts`, `src/deploy.task.ts`,
  and `src/lib/start.task.ts` work with SvelteKit
  ([#157](https://github.com/fuzdev/gro/pull/157),
  [#159](https://github.com/fuzdev/gro/pull/159))
  - add flag `gro deploy --clean` to reset deployment state
  - add flag `--branch` to both tasks, default to `main`
  - default to the `deploy` branch instead of `gh-pages`
- **break**: rename `to_env_string` and `to_env_number` from `stringFromEnv` and `numberFromEnv`
  ([#158](https://github.com/fuzdev/gro/pull/158))
- add helper `read_dir` to `src/lib/node.ts`
  ([#159](https://github.com/fuzdev/gro/pull/159))

## 0.14.0

- **break**: rename `src/lib/node.ts` from `src/lib/nodeFs.ts`
  ([#154](https://github.com/fuzdev/gro/pull/154))
- **break**: rename `src/lib/clean.ts` from `src/lib/clean.ts`
  ([#155](https://github.com/fuzdev/gro/pull/155))
- **break**: rename `to_array` from `ensureArray`
  ([#117](https://github.com/fuzdev/gro/pull/117))
- upgrade to `svelte@3.37.0`
  ([#102](https://github.com/fuzdev/gro/pull/102))
- fix `dist/` build
  ([#156](https://github.com/fuzdev/gro/pull/156))
- export many more things from root: `import {/* !!! */} from '@fuzdev/gro';`
  ([#117](https://github.com/fuzdev/gro/pull/117))
- improve logging readability
  ([#152](https://github.com/fuzdev/gro/pull/152))

## 0.13.0

- **break**: require Node >=14.16.0
  ([#150](https://github.com/fuzdev/gro/pull/150))
- fix false positive Node server detection in default config
  ([#151](https://github.com/fuzdev/gro/pull/151))
- add `get_mime_types` and `get_extensions` returning iterators to `src/lib/mime.ts`
  ([#149](https://github.com/fuzdev/gro/pull/149))
- improve default asset paths to use registered mime types
  ([#149](https://github.com/fuzdev/gro/pull/149))

## 0.12.1

- fix swapped Node server ports
  ([#148](https://github.com/fuzdev/gro/pull/148))

## 0.12.0

- **break**: output builds to `dist/` mirroring the source tree as much as possible
  ([#147](https://github.com/fuzdev/gro/pull/147))
- **break**: rename some things around the Node server
  ([#147](https://github.com/fuzdev/gro/pull/147))
- add `gro start` as an alias for `npm start`
  ([#147](https://github.com/fuzdev/gro/pull/147))
- integrate the default Gro Node server with SvelteKit and Gro's build task
  ([#145](https://github.com/fuzdev/gro/pull/145))
- add task `gro server` to support the default Node server use case in SvelteKit
  ([#145](https://github.com/fuzdev/gro/pull/145))
- remove the `gro dist` task; it's now part of `gro build`
  ([#146](https://github.com/fuzdev/gro/pull/146))

## 0.11.3

- add `events` to `TaskContext` and its generic type to `Task`,
  so tasks can communicate with each other using a normal Node `EventEmitter`
  ([#143](https://github.com/fuzdev/gro/pull/143))
  - events aren't great for everything;
    this PR also documents a value mapping pattern convention for tasks in `src/lib/README.md`
- `gro build` now correctly builds only `BuildConfig`s that have `dist: true`,
  allowing users to customize the `dist/` output in each `gro build` via `gro.config.ts`
  ([#144](https://github.com/fuzdev/gro/pull/144))

## 0.11.2

- add a generic parameter to `Task` to type its `TaskArgs`
  ([#142](https://github.com/fuzdev/gro/pull/142))

## 0.11.1

- track and clean up child processes in `src/lib/process.ts` helpers
  and expose the functionality to users as `registerGlobalSpawn`
  ([#141](https://github.com/fuzdev/gro/pull/141))
- add a confirmation prompt to `gro version`
  with version validations between `changelog.md` and `package.json`
  ([#140](https://github.com/fuzdev/gro/pull/140))

## 0.11.0

- **break**: update dependencies
  ([#139](https://github.com/fuzdev/gro/pull/139))

## 0.10.1

- add the `Filer` option `filter` with type `PathFilter`
  and ignore directories like `.git` by default
  ([#138](https://github.com/fuzdev/gro/pull/138))

## 0.10.0

- **break**: change `gro.config.ts` to export a `config` identifer instead of `default`
  ([#137](https://github.com/fuzdev/gro/pull/137))

## 0.9.5

- set `process.env.NODE_ENV` when running tasks with explicit `dev` values
  ([#136](https://github.com/fuzdev/gro/pull/136))
- remove `import.meta.-env.DEV` support for now despite SvelteKit alignment
  ([#136](https://github.com/fuzdev/gro/pull/136))

## 0.9.4

- forward the optional `dev` arg through the task invocation tree via `invoke_task` and `run_task`
  ([#135](https://github.com/fuzdev/gro/pull/135))

## 0.9.3

- add the optional `dev` property to `Task` definitions to fix the inherited context
  ([#134](https://github.com/fuzdev/gro/pull/134))

## 0.9.2

- fix `src/lib/build.task.ts` ([#133](https://github.com/fuzdev/gro/pull/133)):
  - forward dev arg to esbuild options builder
  - fix return values of rollup plugins to respect sourcemaps

## 0.9.1

- fix `src/lib/build.task.ts` to correctly use `sourcemap` and `target` Gro config options
  ([#132](https://github.com/fuzdev/gro/pull/132))
- turn sourcemaps off by default in production
  ([#132](https://github.com/fuzdev/gro/pull/132))

## 0.9.0

- **break**: separate the default server and primary Node builds
  in `src/lib/gro.config.default.ts`
  ([#131](https://github.com/fuzdev/gro/pull/131))
- **break**: rename `src/lib/toObtainable.ts` to `src/lib/obtainable.ts` to fit convention
  ([#131](https://github.com/fuzdev/gro/pull/131))
- add server auto-restart to `src/lib/dev.task.ts`
  ([#129](https://github.com/fuzdev/gro/pull/129))
- improve the [clean task](https://github.com/fuzdev/gro/blob/main/src/lib/clean.task.ts)
  ([#130](https://github.com/fuzdev/gro/pull/130))
  - running `gro clean` with no options behaves the same, deleting `/.gro/` and `/dist/`
  - `gro clean` now accepts a number of options:
    - `-s`: delete `/.svelte/`
    - `-n`: delete `/node_modules/`
    - `-B`: preserve `/.gro/`, the Gro build directory
    - `-D`: preserve `/dist/`

## 0.8.4

- add `src/lib/version.task.ts` to automate versioning and publishing
  ([#127](https://github.com/fuzdev/gro/pull/127))
- change `src/lib/build.task.ts` to work internally for Gro
  ([#128](https://github.com/fuzdev/gro/pull/128))

## 0.8.3

- change type deps exposed to users to regular dependencies
  ([#126](https://github.com/fuzdev/gro/pull/126))

## 0.8.2

- change `Filer` to extend `EventEmitter` and emit a `'build'` event
  ([#125](https://github.com/fuzdev/gro/pull/125))

## 0.8.1

- fix build not filtering out `null` inputs
  ([#124](https://github.com/fuzdev/gro/pull/124))

## 0.8.0

- **break**: redesign prod build task `src/lib/build.task.ts`
  and remove support for CLI build inputs in favor of config
  ([#123](https://github.com/fuzdev/gro/pull/123))
- internal rename of `src/globalTypes.ts` to `src/globals.d.ts`
  now that they only declare module types
  ([#120](https://github.com/fuzdev/gro/pull/120))

## 0.7.2

- gracefully handle deprecated Gro frontends
  ([#118](https://github.com/fuzdev/gro/pull/118))
- detect default server build at `src/server/server.ts`
  ([#119](https://github.com/fuzdev/gro/pull/119))

## 0.7.1

- rename `src/types.ts` to `src/lib/types.ts`
  ([#115](https://github.com/fuzdev/gro/pull/115))
- add `unwrap` helper to `src/lib/types.ts`
  ([#116](https://github.com/fuzdev/gro/pull/116))

## 0.7.0

- **break**: change global types to root exports
  ([#114](https://github.com/fuzdev/gro/pull/114))

## 0.6.5

- improve the error message when a build config input is missing
  ([#113](https://github.com/fuzdev/gro/pull/113))

## 0.6.4

- add `args` hooks to `src/lib/dev.task.ts`
  ([#112](https://github.com/fuzdev/gro/pull/112))

## 0.6.3

- fix import path
  ([#111](https://github.com/fuzdev/gro/pull/111))
- restrict the dev server to development usage
  ([#107](https://github.com/fuzdev/gro/pull/107))

## 0.6.2

- allow deleting input files
  ([#105](https://github.com/fuzdev/gro/pull/105))

## 0.6.1

- temporarily pin `esinstall` version to fix a breaking regression

## 0.6.0

- replace Gro's internal testing library `oki` with
  [`uvu`](https://github.com/lukeed/uvu)
  ([#103](https://github.com/fuzdev/gro/pull/103))
- add http2 and https support to the dev server
  ([#104](https://github.com/fuzdev/gro/pull/104))

## 0.5.0

- change the `build/` directory to `.gro/` and support multiple builds
  ([#59](https://github.com/fuzdev/gro/pull/59))
- add support for a config file at `gro.config.ts` for custom builds
  ([#67](https://github.com/fuzdev/gro/pull/67),
  [#68](https://github.com/fuzdev/gro/pull/68),
  [#82](https://github.com/fuzdev/gro/pull/82),
  [#83](https://github.com/fuzdev/gro/pull/83),
  [#95](https://github.com/fuzdev/gro/pull/95))
- add `Filer` to replace `CachingCompiler` with additional filesystem capabilities
  ([#54](https://github.com/fuzdev/gro/pull/54),
  [#55](https://github.com/fuzdev/gro/pull/55),
  [#58](https://github.com/fuzdev/gro/pull/58),
  [#60](https://github.com/fuzdev/gro/pull/60),
  [#62](https://github.com/fuzdev/gro/pull/62),
  [#63](https://github.com/fuzdev/gro/pull/63),
  [#94](https://github.com/fuzdev/gro/pull/94),
  [#98](https://github.com/fuzdev/gro/pull/98),
  [#99](https://github.com/fuzdev/gro/pull/99),
  [#100](https://github.com/fuzdev/gro/pull/100),
  [#101](https://github.com/fuzdev/gro/pull/101))
- add Svelte compilation to the unbundled compilation strategies
  ([#52](https://github.com/fuzdev/gro/pull/52),
  [#56](https://github.com/fuzdev/gro/pull/56),
  [#65](https://github.com/fuzdev/gro/pull/65),
  [#66](https://github.com/fuzdev/gro/pull/66))
- bundle external modules for the browser
  ([#61](https://github.com/fuzdev/gro/pull/61),
  [#71](https://github.com/fuzdev/gro/pull/71),
  [#76](https://github.com/fuzdev/gro/pull/76),
  [#81](https://github.com/fuzdev/gro/pull/81),
  [#88](https://github.com/fuzdev/gro/pull/88),
  [#89](https://github.com/fuzdev/gro/pull/89))
- replace swc with esbuild
  ([#92](https://github.com/fuzdev/gro/pull/92))
- make `createBuilder` pluggable allowing users to provide a compiler for each file
  ([#57](https://github.com/fuzdev/gro/pull/57))
- rename `compiler` to `builder`
  ([#70](https://github.com/fuzdev/gro/pull/70))
- replace deep equality helpers with `dequal`
  ([#73](https://github.com/fuzdev/gro/pull/73))
- add a basic client to view project data
  ([#86](https://github.com/fuzdev/gro/pull/86),
  [#87](https://github.com/fuzdev/gro/pull/87),
  [#90](https://github.com/fuzdev/gro/pull/90))
- move terminal color module to src/lib/util
  ([#93](https://github.com/fuzdev/gro/pull/93))
- add server caching
  ([#77](https://github.com/fuzdev/gro/pull/77))

## 0.4.0

- add `swc` dependency along with a Rollup plugin and Svelte preprocessor
  ([#45](https://github.com/fuzdev/gro/pull/45))
- add the `compile` task and use `swc` for non-watchmode builds
  ([#46](https://github.com/fuzdev/gro/pull/46))
- add `CachingCompiler` which uses `swc` to replace `tsc` watchmode
  ([#51](https://github.com/fuzdev/gro/pull/51))
- add stop function return value to `Timings#start`
  ([#47](https://github.com/fuzdev/gro/pull/47))
- rename identifiers from "ext" to "extension" to follow newer convention
  ([#48](https://github.com/fuzdev/gro/pull/48))
- convert `AssertionOperator` from an enum to a union of string types
  ([#49](https://github.com/fuzdev/gro/pull/49))
- rename `AsyncState` to `AsyncStatus` and convert it from an enum to a union of string types
  ([#50](https://github.com/fuzdev/gro/pull/50))

## 0.3.0

- handle build errors in the deploy task and add the `--dry` deploy flag
  ([#42](https://github.com/fuzdev/gro/pull/42))
- upgrade dependencies
  ([#43](https://github.com/fuzdev/gro/pull/43),
  [#44](https://github.com/fuzdev/gro/pull/44))

## 0.2.12

- fix log message when listing all tasks
  ([#41](https://github.com/fuzdev/gro/pull/41))

## 0.2.11

- change the deploy task to delete `dist/` when done to avoid git worktree issues
  ([#40](https://github.com/fuzdev/gro/pull/40))

## 0.2.10

- add a default `gro deploy` task for GitHub pages
  ([#39](https://github.com/fuzdev/gro/pull/39))
- run the clean task at the beginning of the check task
  ([#37](https://github.com/fuzdev/gro/pull/37))

## 0.2.9

- sort CSS builds to make output deterministic
  ([#36](https://github.com/fuzdev/gro/pull/36))

## 0.2.8

- make Rollup build extensible
  ([#35](https://github.com/fuzdev/gro/pull/35))
- upgrade peer dependencies
  ([#34](https://github.com/fuzdev/gro/pull/34))

## 0.2.7

- enable sourcemaps for build in development mode
  ([#33](https://github.com/fuzdev/gro/pull/33))

## 0.2.6

- add `uuid` utilities
  ([#31](https://github.com/fuzdev/gro/pull/31))

## 0.2.5

- add `randomFloat` utility
  ([#30](https://github.com/fuzdev/gro/pull/30))

## 0.2.4

- add `Result` type helper to `src/globalTypes.ts`
  ([#29](https://github.com/fuzdev/gro/pull/29))

## 0.2.3

- fix external module type declarations by merging
  `src/lib/globalTypes.d.ts` into `src/globalTypes.ts`
  ([#28](https://github.com/fuzdev/gro/pull/28))

## 0.2.2

- export `kleur/colors` from `src/lib/terminal.js`
  ([#27](https://github.com/fuzdev/gro/pull/27))

## 0.2.1

- add type helpers `Branded` and `Flavored` for nominal-ish typing
  ([#23](https://github.com/fuzdev/gro/pull/23))

## 0.2.0

- **breaking:** upgrade `kleur` dep and remove color wrappers
  ([#26](https://github.com/fuzdev/gro/pull/26))

## 0.1.14

- correctly fix `.js` module resolution where
  [#24](https://github.com/fuzdev/gro/pull/24) failed
  ([#25](https://github.com/fuzdev/gro/pull/25))

## 0.1.13

- change assertions `t.is` and `t.equal` to use a shared generic type for extra safety
  ([#22](https://github.com/fuzdev/gro/pull/22))
- fix `.js` module resolution in the Rollup TypeScript plugin
  ([#24](https://github.com/fuzdev/gro/pull/24))

## 0.1.12

- add the `invoke_task` helper for task composition
  ([#20](https://github.com/fuzdev/gro/pull/20))
- add CLI flags to print the Gro version with `--version` or `-v`
  ([#21](https://github.com/fuzdev/gro/pull/21))

## 0.1.11

- fix `terser` import
- export `Unobtain` type from `util/obtainable.ts`

## 0.1.10

- add async `rejects` assertion
  ([#19](https://github.com/fuzdev/gro/pull/19))
- change the check task to run tests only if some exist
  ([#18](https://github.com/fuzdev/gro/pull/18))

## 0.1.9

- actually fix unbuilt project detection when invoking builtin Gro tasks
  ([#17](https://github.com/fuzdev/gro/pull/17))

## 0.1.8

- fix unbuilt project detection when invoking builtin Gro tasks
  ([#16](https://github.com/fuzdev/gro/pull/16))

## 0.1.7

- compile TypeScript if an invoked task cannot be found in `build/`
  ([#12](https://github.com/fuzdev/gro/pull/12))
- change the check task to look for stale generated files only if the project contains gen files
  ([#13](https://github.com/fuzdev/gro/pull/13))
- format files in the root directory, not just `src/`
  ([#15](https://github.com/fuzdev/gro/pull/15))

## 0.1.6

- change `gro clean` to delete directories instead of emptying them
  ([#11](https://github.com/fuzdev/gro/pull/11))

## 0.1.5

- add `gro format` and `gro format --check` and format generated code
  ([#8](https://github.com/fuzdev/gro/pull/8))
- add `prettier` and `prettier-plugin-svelte` as peer dependencies and upgrade to Prettier 2
  ([#8](https://github.com/fuzdev/gro/pull/8))

## 0.1.4

- ensure the project has been built when invoking tasks
  ([#5](https://github.com/fuzdev/gro/pull/5))

## 0.1.3

- upgrade TypeScript minor version
- rename `util/random.ts` functions, expanding "rand" prefix to "random"

## 0.1.2

- upgrade TypeScript dep
- add `util/obtainable.ts` for decoupled lifecycle management

## 0.1.1

- add `util/watch_dir.ts` for low level filesystem watching
- expose `remove` and `ensure_dir` in `util/node.ts`

## 0.1.0

- plant in the ground
