# dev

Gro is designed to extend [SvelteKit](https://github.com/sveltejs/kit)
with helpful tools. It supports:

- frontends with SvelteKit and [Vite](https://github.com/vitejs/vite)
- Node libraries
- Node servers

> Where it's going: the [runtime](runtime.md) keeps this workflow and moves it onto Gro's own host,
> running Vite and the other Node tools as subprocesses under the run's kernel rules
> until Gro bundles dev and build itself and Vite leaves the loop.

## usage

```bash
gro dev
gro dev --no-watch # outputs dev artifacts and exits without watch mode
```

To configure a project, see [the config docs](config.md).

## plugin

`Plugin`s are objects that customize the behavior of `gro build` and `gro dev`.
See [plugin.md](plugin.md) to learn more.

## todo

- [x] basics
- [ ] add API using esbuild to optionally bundle specific pieces to speed up development
- [ ] livereload CSS (and fix pop-in during dev)
- [ ] HMR
- [ ] probably support Rollup plugins in development, but how?
- [ ] improve loading speed with `cache-control: immutable` and
      [import maps](https://github.com/WICG/import-maps/)
      (on my machine Firefox is much slower than Chrome
      handling a module import waterfall, locally, and http2 didn't help)

<p align="center">
  <a href="https://github.com/fuzdev/gro">
    <img src="static/logo.svg" alt="a pixelated green oak acorn with a glint of sun" width="192" height="192">
  </a>
</p>
