# server plugin

Gro's [server plugin](/src/lib/gro_plugin_server.ts) runs a server during `gro dev`,
beside the Vite dev server, and restarts it when its entry or anything the entry imports changes.

> Where it's going: the entry runs as `gro run` runs a file, so when Gro's own
> [runtime](runtime.md) runs servers, the plugin keeps its options and the server moves with it.

```ts
// gro.config.ts
import type {CreateGroConfig} from '@fuzdev/gro/gro_config.ts';
import {gro_plugin_server} from '@fuzdev/gro/gro_plugin_server.ts';

const config: CreateGroConfig = (cfg) => {
	const base_plugins = cfg.plugins;
	cfg.plugins = async (ctx) => [
		...(await base_plugins(ctx)),
		gro_plugin_server({
			entry: 'src/lib/server/server.ts',
			env: {PORT: '4041', HOST: '127.0.0.1'},
		}),
	];
	return cfg;
};

export default config;
```

Vite proxies API requests to the server through its own `server.proxy` config.

## options

- `entry` - the server's entry module, relative to the project root
- `env_file` - an env file whose vars the server gets, relative to the project root,
  default `.env.development`, `null` for none
- `env` - vars over everything else, like `PORT` and `HOST`
- `watch` - whether changes restart the server, default `true`

## running

The plugin starts the server only in `gro dev`'s watch mode: `gro build` never starts it,
and neither does `gro dev --no-watch`, which tears down right after setup.
At setup it checks that the entry exists and starts it in its own process,
with Gro's loader as `gro run` registers it, so TypeScript, `.svelte.ts` modules,
and the SvelteKit shims like `$app/env/private` work as they do in a task.
Its stdio is the terminal's, so its output and errors print as they would from `gro run`.

Teardown stops the server, `SIGTERM` and then `SIGKILL` after a grace period,
waiting for it to exit. The server shares the terminal, so it isn't a process group of its own,
and a stop or restart signals the server alone: a process it started and left running outlives it.
In watch mode `gro dev` runs no teardown and the server lives as long as Gro:
a `SIGTERM` to Gro passes on to the server, and a terminal's Ctrl-C reaches both.
A `SIGKILL` of Gro leaves the server running.

## environment

The server gets the host's whole environment, since a server reads a set of vars Gro can't know.
Beneath it are the env file's vars, so a var the host sets keeps the host's value,
as a dotenv loader leaves set vars alone. Over both are `NODE_ENV=development` and then `env`,
which can set `NODE_ENV` too. A missing env file warns and loads nothing.
The environment is fixed at setup, so an edit to the env file takes a new `gro dev`.

## restarts

The plugin watches the task context's module graph, which tracks the modules under the config's
`watch.dir` (default `src/`) and their imports by path or `#` subpath.
A change restarts the server when it's the entry itself or a module the entry imports,
directly or transitively, an added or deleted file included, so a file the entry imports
before it exists restarts the server once it's created.
A change outside the entry's imports restarts nothing, and so does a package's file
reached by a bare specifier, or a file outside `watch.dir`.
An entry outside `watch.dir`, or one `watch.filter` rejects, warns at setup, since the graph
doesn't watch it, so a change to it restarts nothing, and a change to what it imports restarts
the server only when a module under `watch.dir` imports the entry too.

Changes are content-based, so saving a file unchanged restarts nothing.
A burst of changes, like a save-all or a branch switch, restarts the server once,
after a short quiet period, and restarts never overlap:
each stops the running server and waits for it to exit before starting the next.

A server that exits on its own, like one that crashes or fails to start from a broken edit,
is logged and stays down until the next change that reaches it, which starts it again,
so a broken edit never loops. A restart that can't launch is logged and retried on the next change.
