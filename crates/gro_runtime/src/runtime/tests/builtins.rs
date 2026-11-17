//! Tests of the builtin modules: each imports from either start.

use super::*;

/// What a module that imports every builtin saw on a runtime started as
/// `start` says, in a run that grants `args`: `ok`, or the report of
/// what failed. The run's arguments are replaced between the runtime's
/// start and the module's load, so the module tells a script run by its
/// import from one the start ran. With it, whether the import made an op's
/// function again: `deno_core` remakes each op that has a fast call the
/// first time code calls `core.loadExtScript` for a script the snapshot
/// lacks, and a builtin's first import doesn't go through that call.
pub(super) fn builtin_imports(start: Start) -> Value {
    use crate::resolver::DiskTree;

    init_engine(Confinement::Unconfined, &engine_options()).unwrap();
    let snapshot = (start == Start::Snapshot).then(child_snapshot);
    let dir = TestDir::on_disk(&format!("builtin_imports_{start:?}"));
    let rows: Vec<(&str, &[&str])> = BUILTINS
        .iter()
        .map(|builtin| (builtin.specifier, builtin.exports))
        .collect();
    let source = format!(
        "const check = (what, ok) => {{ if (!ok) throw new Error(what); }};
const seen = {{}};
for (const [specifier, exports] of {}) {{
\tconst ns = await import(specifier);
\tconst keys = Reflect.ownKeys(ns).filter((key) => typeof key === 'string');
\tcheck(`${{specifier}} exports ${{keys}}`, JSON.stringify(keys) === JSON.stringify([...exports, 'default'].sort()));
\tcheck(`${{specifier}} default`, ns.default === undefined);
\tObject.assign(seen, ns);
}}
check(`args ${{JSON.stringify(seen.args)}}`, JSON.stringify(seen.args) === '[\"a\"]');
check(`cwd ${{seen.cwd()}}`, seen.cwd() === '/work');
check(`pid ${{seen.pid}}`, seen.pid === {});
check(`runtime_version ${{seen.runtime_version}}`, seen.runtime_version === 'test/1.2.3');
const error = new seen.EnvError('not_granted', 'HOME');
check(`EnvError ${{error.message}}`, error.message === 'not_granted: HOME' && error instanceof Error);
check('is_env_error', seen.is_env_error(error, 'not_granted') && !seen.is_env_error(new Error('HOME')));
check('env', seen.env.get('GRO_TEST_ENV') === undefined && Reflect.ownKeys(seen.env.get_all()).length === 0);
try {{
	seen.env.get('A');
	throw new Error('read A');
}} catch (refused) {{
	check(`env refused ${{refused}}`, seen.is_env_error(refused, 'not_granted') && refused.message === 'not_granted: A');
}}
const hex = (bytes) => Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('');
const empty = hex(seen.hash(new Uint8Array()));
check(`hash ${{empty}}`, empty === 'af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262');
const hasher = seen.create_hasher();
hasher.update(new Uint8Array([1, 2]));
hasher.update(new Uint8Array([3]));
check('a hasher', hex(hasher.finalize()) === hex(seen.hash(new Uint8Array([1, 2, 3]))));
check('an op was made again', globalThis.core_ops.op_gro_color === globalThis.op_gro_color);
",
        serde_json::to_string(&rows).unwrap(),
        std::process::id()
    );
    std::fs::write(dir.path().join("main.mjs"), source).unwrap();
    let cwd = dir.path().to_owned();
    let imported = std::thread::Builder::new()
        .stack_size(STACK_SIZE)
        .spawn(move || -> Result<(), String> {
            let (entry, roots) =
                crate::resolver::resolve_entry(&DiskTree, Path::new("main.mjs"), &cwd).unwrap();
            let authority = crate::test_support::run_authority();
            let loader = Loader::new(&entry, roots, Recording::Off);
            let loader = Rc::new(loader.with_authority(authority.clone()));
            let refusals = loader.refusal_log();
            let tokio = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            tokio.block_on(async {
                let mut runtime = new_runtime(snapshot, loader, Mode::Run, None)?;
                // a fast op's function, and the object the remaking writes to
                runtime
                    .execute_script(
                        "<test>",
                        "globalThis.core_ops = Deno.core.ops;
globalThis.op_gro_color = Deno.core.ops.op_gro_color;",
                    )
                    .map_err(|error| report_js(&error))?;
                let state = runtime.op_state();
                state.borrow_mut().put(authority);
                state.borrow_mut().put(HostData {
                    args: vec!["the start's".to_owned()],
                    ..host_data()
                });
                state.borrow_mut().put(ErrorClasses::default());
                state.borrow_mut().put(NativeNamespaces::default());
                state.borrow_mut().put(Blake3State::default());
                state.borrow_mut().put(refusals);
                let mut runtime =
                    super::super::start(runtime, ConsoleOptions::default(), Mode::Run)?;
                state.borrow_mut().put(host_data());
                let id = runtime
                    .load_main_es_module(&entry.url)
                    .await
                    .map_err(|error| report(&error).to_string())?;
                let evaluation = runtime.mod_evaluate(id);
                runtime
                    .run_event_loop(deno_core::PollEventLoopOptions::default())
                    .await
                    .map_err(|error| report(&error).to_string())?;
                evaluation.await.map_err(|error| report(&error).to_string())
            })
        })
        .unwrap()
        .join()
        .unwrap();
    json!({
        "imported": imported.err().unwrap_or_else(|| "ok".to_owned()),
    })
}

/// Every builtin imports from either start with the run's values, its
/// script run by its first import, never by the start, with the ops the
/// runtime's code already holds.
#[test]
fn a_builtin_s_script_runs_at_its_first_import_from_either_start() {
    for start in STARTS {
        assert_eq!(
            probed("builtin_imports", start)["imported"],
            "ok",
            "{start:?}"
        );
    }
}
