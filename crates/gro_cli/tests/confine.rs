//! The box from inside a confined process: each case runs in a child, this
//! binary run again with the case's name, which confines itself from its main
//! thread, its only one, as `fgro eval` does, and prints what it then could
//! and couldn't do; the parent reads its exit status and output.
//!
//! The binary runs without libtest's harness, whose threads would refuse the
//! box. It needs a snapshot build, for the case that evaluates a module
//! under the box, and Linux, the only platform with one, so it runs from
//! `cargo xtask build --verify`; elsewhere it does nothing.
//!
//! Built with the `prices` feature too, it adds a case that installs
//! Landlock without the seccomp allowlist (`GRO_PRICES_OFF=seccomp`), whose
//! `ENOSYS` for every socket call otherwise answers before Landlock's TCP
//! and abstract unix socket walls can.
//!
//! On a kernel whose Landlock is below the ABI the box requires, every case
//! fails, naming the kernel's ABI: confined eval refuses to run there, so its
//! tests don't pass there either.

// clippy's test allowance covers `#[test]` functions, and this binary has none
#![cfg_attr(
    all(feature = "snapshot", target_os = "linux"),
    expect(
        clippy::unwrap_used,
        clippy::panic,
        reason = "a test's setup and its failures panic"
    )
)]

fn main() {
    #[cfg(all(feature = "snapshot", target_os = "linux"))]
    confined::main();
}

#[cfg(all(feature = "snapshot", target_os = "linux"))]
mod confined {
    use std::io::ErrorKind;
    use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
    use std::os::unix::process::{CommandExt as _, ExitStatusExt as _};
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::Duration;

    use gro_runtime::STACK_SIZE;
    use gro_runtime::confine::{ConfineError, Confinement, Jit, Prepared};
    use gro_runtime::engine::init_confined_engine;
    use gro_runtime::eval::{EvalOptions, RuntimeInfo, engine_options};
    use gro_runtime::snapshot::StartupSnapshot;

    /// The snapshot the binary under test embeds, as `fgro` embeds it.
    mod embedded {
        pub static SNAPSHOT: &[u8] =
            include_bytes!(concat!(env!("GRO_SNAPSHOT_DIR"), "/snapshot.bin"));
        include!(concat!(env!("GRO_SNAPSHOT_DIR"), "/residual.rs"));
    }

    /// The variable naming the defaults a priced build turns off,
    /// `gro_runtime::prices`'s.
    const PRICES_OFF_ENV: &str = "GRO_PRICES_OFF";

    /// The signal seccomp kills a process with.
    const SIGSYS: i32 = 31;

    /// The signal the signal case sends a process outside the box, harmless
    /// to a sleeping one, the same on every architecture the box is built
    /// for.
    const SIGCONT: i32 = 18;

    /// `open`'s truncate flag, the same on every architecture the box is
    /// built for.
    const O_TRUNC: i32 = 0o1000;

    /// The kernel crypto API's address family, which std can't make a socket
    /// of.
    const AF_ALG: i32 = 38;

    /// The virtual machine sockets' address family, which std can't make a
    /// socket of.
    const AF_VSOCK: i32 = 40;

    /// The error a call fails with when the box's allowlist doesn't allow it,
    /// the same on every architecture the box is built for.
    const ENOSYS: i32 = 38;

    /// `mmap`'s and `mprotect`'s flags for a readable, writable, and
    /// executable mapping, the same on every architecture the box is built
    /// for.
    const PROT_READ: i32 = 1;
    const PROT_WRITE: i32 = 2;
    const PROT_EXEC: i32 = 4;

    /// The cases, each run in a child of its own; a priced build adds the
    /// one that turns the allowlist off.
    const CASES: &[&str] = &[
        "reads",
        "exec",
        "sockets",
        "signal",
        "process",
        "memory",
        "memory_jit",
        "threads",
        "eval",
        #[cfg(feature = "prices")]
        "landlock_sockets",
    ];

    pub fn main() {
        let mut args = std::env::args().skip(1);
        match args.next().as_deref() {
            Some("child") => {
                let case = args.next().unwrap();
                let scratch = PathBuf::from(args.next().unwrap());
                child(&case, &scratch);
            }
            // libtest's own flags (`--test-threads`, a filter) are ignored
            _ => parent(),
        }
    }

    /// The scratch directory: a project, with a module, and a file outside it.
    fn scratch() -> PathBuf {
        let dir =
            Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("confine_{}", std::process::id()));
        for (file, content) in [
            ("project/package.json", r#"{"name": "p", "type": "module"}"#),
            ("project/main.js", "export default {box: 'held'};\n"),
            ("project/truncated.txt", "kept\n"),
            ("outside/secret.txt", "outside\n"),
        ] {
            let path = dir.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
        dir.canonicalize().unwrap()
    }

    /// The file the reads case tries to create in `/tmp`, named by the
    /// scratch directory, so the parent can remove it if one was.
    fn tmp_file(scratch: &Path) -> PathBuf {
        Path::new("/tmp").join(format!("gro_{}", scratch.file_name().unwrap().display()))
    }

    /// Runs every case in a child and checks what it reports.
    fn parent() {
        let scratch = scratch();
        let mut failures = Vec::new();
        for &case in CASES {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args(["child", case])
                .arg(&scratch)
                // eval refuses any other time zone, and ICU reads the host's
                .env("TZ", "UTC")
                // a priced build's switches, read only with its feature
                .env_remove(PRICES_OFF_ENV);
            if case == "landlock_sockets" {
                command.env(PRICES_OFF_ENV, "seccomp");
            }
            let output = command.output().unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            // the signal case's process outside the box
            if let Some(victim) = stderr.lines().find_map(|line| line.strip_prefix("victim ")) {
                let _ = Command::new("kill").arg(victim).status();
            }
            let result = check(case, output.status, &stdout);
            println!(
                "test {case} ... {}",
                if result.is_ok() { "ok" } else { "FAILED" }
            );
            if let Err(why) = result {
                failures.push(format!(
                    "{case}: {why}\n  status: {}\n  stdout: {stdout}\n  stderr: {stderr}",
                    output.status
                ));
            }
        }
        // a file a breached box created in `/tmp`
        let _ = std::fs::remove_file(tmp_file(&scratch));
        std::fs::remove_dir_all(&scratch).unwrap();
        assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    }

    /// Checks what the child running `case` exited with and printed.
    fn check(case: &str, status: std::process::ExitStatus, stdout: &str) -> Result<(), String> {
        let expected: &[&str] = match case {
            // seccomp kills the process at its execve
            "exec" => {
                return if status.signal() == Some(SIGSYS) && stdout == "confined\n" {
                    Ok(())
                } else {
                    Err(format!(
                        "expected a kill by SIGSYS after confining, got {status}"
                    ))
                };
            }
            "reads" => &[
                "confined",
                "read inside: ok",
                "read outside: denied",
                "read /proc/self/status: denied",
                "read /dev/urandom: denied",
                "write inside: denied",
                "create outside: denied",
                "outside after: absent",
                "create /tmp: denied",
                "/tmp after: absent",
                "write outside: denied",
                "append outside: denied",
                "truncate inside: denied",
                "chmod inside: enosys",
                "fchmod inside: enosys",
                "chown inside: enosys",
                "utimensat inside: enosys",
                "flock inside: enosys",
                "dup inside: ok",
                "available_parallelism: ok",
                "read outside from a thread: denied",
                "read outside from its thread: denied",
            ],
            "sockets" => &[
                "confined",
                "tcp: enosys",
                "udp6: enosys",
                "unix: enosys",
                "vsock: enosys",
                "alg: enosys",
                "socketpair: ok",
                "socketpair nonblocking: ok",
                "socketpair dgram: enosys",
                "bind abstract: enosys",
                "listen: enosys",
                "tcp connect made before: enosys",
                "connect abstract made before: enosys",
            ],
            "landlock_sockets" => &[
                "confined",
                "tcp bind: denied",
                "tcp connect: denied",
                "connect abstract: denied",
                "tcp connect made before: denied",
                "connect abstract made before: denied",
                "bind abstract inside: ok",
                "connect abstract inside: ok",
            ],
            "signal" => &[
                "confined",
                "kill outside: enosys",
                "tgkill outside: denied",
                "tgkill self: ok",
            ],
            "process" => &[
                "confined",
                "thread: ok",
                "spawn: enosys",
                "read data limit: enosys",
                "set data limit: enosys",
            ],
            "memory" => &[
                "confined",
                "mmap read write: ok",
                "mmap exec: enosys",
                "mmap read exec: enosys",
                "mprotect read: ok",
                "mprotect exec: enosys",
                "mprotect read write exec: enosys",
            ],
            "memory_jit" => &[
                "confined",
                "mmap read write: ok",
                "mmap exec: ok",
                "mmap read exec: ok",
                "mprotect read: ok",
                "mprotect exec: ok",
                "mprotect read write exec: ok",
            ],
            "threads" => &["refused: 2 threads", "read outside: ok"],
            "eval" => &[
                "confined",
                r#"value: {"box":"held"}"#,
                "read outside from the eval's thread: denied",
            ],
            _ => return Err(format!("no case {case}")),
        };
        let expected = format!("{}\n", expected.join("\n"));
        if status.success() && stdout == expected {
            Ok(())
        } else {
            Err(format!("expected exit 0 with\n{expected}"))
        }
    }

    /// Confines this process to `project`, for an engine whose JIT is `jit`,
    /// or prints why it can't and exits.
    fn confine(project: &Path, jit: Jit) -> Confinement {
        match Prepared::for_project(project.to_owned())
            .unwrap()
            .confine(jit)
        {
            Ok(confined) => {
                println!("confined");
                Confinement::Confined(confined)
            }
            Err(error) => {
                println!("refused: {error}");
                std::process::exit(3);
            }
        }
    }

    /// `ok`, or `denied` for a permission error (Landlock's), or `enosys`
    /// for a call the allowlist doesn't allow, or the error.
    fn outcome<T>(result: std::io::Result<T>) -> String {
        match result {
            Ok(_) => "ok".to_owned(),
            Err(error) if error.kind() == ErrorKind::PermissionDenied => "denied".to_owned(),
            Err(error) if error.raw_os_error() == Some(ENOSYS) => "enosys".to_owned(),
            Err(error) => format!("failed: {error}"),
        }
    }

    /// Runs `case` in this process, the child.
    fn child(case: &str, scratch: &Path) {
        let project = scratch.join("project");
        let outside = scratch.join("outside/secret.txt");
        match case {
            "reads" => {
                confine(&project, Jit::Off);
                let read = |path: &Path| outcome(std::fs::read(path));
                println!("read inside: {}", read(&project.join("main.js")));
                println!("read outside: {}", read(&outside));
                println!(
                    "read /proc/self/status: {}",
                    read(Path::new("/proc/self/status"))
                );
                println!(
                    "read /dev/urandom: {}",
                    outcome(std::fs::File::open("/dev/urandom"))
                );
                println!(
                    "write inside: {}",
                    outcome(std::fs::write(project.join("written.txt"), ""))
                );
                // outside the roots: a file created beside the project and in
                // `/tmp`, and one that exists opened to write and to append;
                // whether a path exists stays readable (`statx`), so nothing
                // was created
                let tmp = tmp_file(scratch);
                for (name, path) in [
                    ("outside", scratch.join("outside/written.txt")),
                    ("/tmp", tmp),
                ] {
                    println!(
                        "create {name}: {}",
                        outcome(
                            std::fs::OpenOptions::new()
                                .write(true)
                                .create_new(true)
                                .open(&path)
                        )
                    );
                    println!(
                        "{name} after: {}",
                        match std::fs::symlink_metadata(&path) {
                            Ok(_) => "present".to_owned(),
                            Err(error) if error.kind() == ErrorKind::NotFound =>
                                "absent".to_owned(),
                            Err(error) => format!("failed: {error}"),
                        }
                    );
                }
                println!(
                    "write outside: {}",
                    outcome(std::fs::OpenOptions::new().write(true).open(&outside))
                );
                println!(
                    "append outside: {}",
                    outcome(std::fs::OpenOptions::new().append(true).open(&outside))
                );
                // opened to read, which the box allows, with `O_TRUNC`, which
                // Landlock gates from ABI 3
                println!(
                    "truncate inside: {}",
                    outcome(
                        std::fs::OpenOptions::new()
                            .read(true)
                            .custom_flags(O_TRUNC)
                            .open(project.join("truncated.txt"))
                    )
                );
                // a file's metadata, which Landlock at ABI 6 doesn't gate and
                // the allowlist doesn't allow: its mode set to the one it has,
                // by path and by handle, its owner to the one it has, and its
                // times
                let main = project.join("main.js");
                let metadata = std::fs::metadata(&main).unwrap();
                println!(
                    "chmod inside: {}",
                    outcome(std::fs::set_permissions(&main, metadata.permissions()))
                );
                println!(
                    "fchmod inside: {}",
                    outcome(
                        std::fs::File::open(&main)
                            .and_then(|file| file.set_permissions(metadata.permissions()))
                    )
                );
                println!(
                    "chown inside: {}",
                    outcome(std::os::unix::fs::chown(
                        &main,
                        Some(metadata.uid()),
                        Some(metadata.gid())
                    ))
                );
                println!(
                    "utimensat inside: {}",
                    outcome(
                        std::fs::File::open(&main)
                            .and_then(|file| file.set_modified(metadata.modified()?))
                    )
                );
                // an advisory lock (`flock`), which the allowlist refuses, and
                // a duplicate handle (`fcntl`'s `F_DUPFD_CLOEXEC`), which it
                // keeps
                println!(
                    "flock inside: {}",
                    outcome(std::fs::File::open(&main).and_then(|file| file.lock()))
                );
                println!(
                    "dup inside: {}",
                    outcome(std::fs::File::open(&main).and_then(|file| file.try_clone()))
                );
                // std's fallback when the cgroup and CPU files are denied
                println!(
                    "available_parallelism: {}",
                    outcome(std::thread::available_parallelism())
                );
                std::thread::spawn(move || {
                    println!(
                        "read outside from a thread: {}",
                        outcome(std::fs::read(&outside))
                    );
                    std::thread::spawn(move || {
                        println!(
                            "read outside from its thread: {}",
                            outcome(std::fs::read(&outside))
                        );
                    })
                    .join()
                    .unwrap();
                })
                .join()
                .unwrap();
            }
            "exec" => {
                confine(&project, Jit::Off);
                let error = Command::new("/bin/true").exec();
                println!("exec returned: {error}");
            }
            "sockets" => {
                let outside_box = Listeners::new();
                confine(&project, Jit::Off);
                println!(
                    "tcp: {}",
                    outcome(std::net::TcpListener::bind("127.0.0.1:0"))
                );
                println!("udp6: {}", outcome(std::net::UdpSocket::bind("[::1]:0")));
                println!(
                    "unix: {}",
                    outcome(std::os::unix::net::UnixDatagram::unbound())
                );
                // families std can't make: no `socket` call is allowed
                for (name, family, kind) in [
                    ("vsock", AF_VSOCK, socket2::Type::STREAM),
                    ("alg", AF_ALG, socket2::Type::SEQPACKET),
                ] {
                    println!(
                        "{name}: {}",
                        outcome(socket2::Socket::new(
                            socket2::Domain::from(family),
                            kind,
                            None
                        ))
                    );
                }
                println!(
                    "socketpair: {}",
                    outcome(std::os::unix::net::UnixStream::pair())
                );
                // made non-blocking as tokio makes it, by `ioctl`'s `FIONBIO`
                println!(
                    "socketpair nonblocking: {}",
                    outcome(
                        std::os::unix::net::UnixStream::pair()
                            .and_then(|(kept, _peer)| kept.set_nonblocking(true))
                    )
                );
                // a datagram pair could send to any socket file by its path
                println!(
                    "socketpair dgram: {}",
                    outcome(std::os::unix::net::UnixDatagram::pair())
                );
                // the kept stream pair's socket, bound to an abstract name or
                // listening on one
                let (kept, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
                let kept = socket2::Socket::from(kept);
                println!(
                    "bind abstract: {}",
                    outcome(
                        socket2::SockAddr::unix("\0gro_confine_bind")
                            .and_then(|name| kept.bind(&name))
                    )
                );
                println!("listen: {}", outcome(kept.listen(1)));
                // `connect`, by sockets made before the box, to listeners
                // outside it
                outside_box.connect();
            }
            // Landlock alone, the allowlist turned off (a priced build's
            // `seccomp`), so its own walls answer what the allowlist's
            // `ENOSYS` hides: no TCP bind or connect, and no connect to an
            // abstract unix socket made outside the box, while one made
            // inside it takes a connect
            #[cfg(feature = "prices")]
            "landlock_sockets" => {
                let outside_box = Listeners::new();
                confine(&project, Jit::Off);
                println!(
                    "tcp bind: {}",
                    outcome(std::net::TcpListener::bind("127.0.0.1:0"))
                );
                println!(
                    "tcp connect: {}",
                    outcome(std::net::TcpStream::connect(outside_box.tcp_address))
                );
                println!(
                    "connect abstract: {}",
                    outcome(std::os::unix::net::UnixStream::connect_addr(
                        &outside_box.unix_address
                    ))
                );
                outside_box.connect();
                let inside = abstract_address("in");
                // the listener held while the connect is tried
                let listener = std::os::unix::net::UnixListener::bind_addr(&inside);
                let connected = std::os::unix::net::UnixStream::connect_addr(&inside);
                println!("bind abstract inside: {}", outcome(listener));
                println!("connect abstract inside: {}", outcome(connected));
            }
            "signal" => {
                // a process outside the box, started before it; the parent
                // stops it, which this process no longer can
                // its stdio null, so the parent's read of this child's pipes
                // doesn't wait for it
                #[expect(
                    clippy::zombie_processes,
                    reason = "this process can't stop it once boxed; the parent kills it, and \
                              it is reaped by whichever process inherits it once this one exits"
                )]
                let mut victim = Command::new("sleep")
                    .arg("60")
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .unwrap();
                eprintln!("victim {}", victim.id());
                let victim_id = i32::try_from(victim.id()).unwrap();
                confine(&project, Jit::Off);
                // `kill`, std's call, which the allowlist refuses
                println!("kill outside: {}", outcome(victim.kill()));
                // `tgkill`, which the allowlist keeps for `abort()`, so only
                // Landlock's signal scope refuses it, to the victim's one
                // thread, whose id is its process's
                println!(
                    "tgkill outside: {}",
                    outcome(gro_sys::signal_thread(victim_id, victim_id, SIGCONT))
                );
                // this process's own thread, the main one, whose id is the
                // process's, with signal 0, which sends nothing
                let own = i32::try_from(std::process::id()).unwrap();
                println!(
                    "tgkill self: {}",
                    outcome(gro_sys::signal_thread(own, own, 0))
                );
            }
            "process" => {
                confine(&project, Jit::Off);
                // glibc starts a thread with `clone3` first, which the
                // allowlist refuses, then with `clone`, which it allows for a
                // thread: the thread's start is what's checked, since its
                // name, set with `prctl`, can't be read back from inside the
                // box (`PR_GET_NAME` is refused, and `/proc` is outside it)
                let main = project.join("main.js");
                std::thread::spawn(move || {
                    println!("thread: {}", outcome(std::fs::read(&main)));
                })
                .join()
                .unwrap();
                // a process forked or spawned is refused before anything is
                // executed, so the killing filter's `execve` is never reached
                println!("spawn: {}", outcome(Command::new("/bin/true").status()));
                // the process's limits can't be read or changed: the
                // allowlist refuses `getrlimit`, `setrlimit`, and
                // `prlimit64`, so the memory cap `fgro eval` sets before the
                // box is never raised (`limit_data` reads before it sets)
                println!("read data limit: {}", outcome(gro_sys::data_limit()));
                let set = gro_sys::limit_data(u64::MAX);
                println!(
                    "set data limit: {}",
                    match set {
                        Err(error) if error.kind() == ErrorKind::Unsupported => "enosys".to_owned(),
                        other => outcome(other),
                    }
                );
            }
            // the jitless box refuses an executable mapping, made or changed
            // to, and allows the rest; the JIT's box allows both
            "memory" | "memory_jit" => {
                let jit = if case == "memory" { Jit::Off } else { Jit::On };
                confine(&project, jit);
                for (name, prot) in [
                    ("read write", PROT_READ | PROT_WRITE),
                    ("exec", PROT_EXEC),
                    ("read exec", PROT_READ | PROT_EXEC),
                ] {
                    println!("mmap {name}: {}", outcome(gro_sys::map_anonymous(prot)));
                }
                for (name, prot) in [
                    ("read", PROT_READ),
                    ("exec", PROT_EXEC),
                    ("read write exec", PROT_READ | PROT_WRITE | PROT_EXEC),
                ] {
                    println!(
                        "mprotect {name}: {}",
                        outcome(gro_sys::protect_anonymous(prot))
                    );
                }
            }
            "threads" => {
                let (stop, wait) = std::sync::mpsc::channel::<()>();
                let other = std::thread::spawn(move || wait.recv());
                match Prepared::for_project(project).unwrap().confine(Jit::Off) {
                    Ok(_) => println!("confined with another thread alive"),
                    Err(ConfineError::Threads { count }) => println!("refused: {count} threads"),
                    Err(error) => println!("refused otherwise: {error}"),
                }
                stop.send(()).unwrap();
                other.join().unwrap().unwrap();
                // nothing was installed
                println!("read outside: {}", outcome(std::fs::read(&outside)));
            }
            "eval" => eval(&project, &outside),
            _ => panic!("no case {case}"),
        }
    }

    /// An abstract unix socket address unique to this process, named by
    /// `which`.
    fn abstract_address(which: &str) -> std::os::unix::net::SocketAddr {
        use std::os::linux::net::SocketAddrExt as _;
        let name = format!("gro_confine_{which}_{}", std::process::id());
        std::os::unix::net::SocketAddr::from_abstract_name(name).unwrap()
    }

    /// Listeners made before the box, so outside it: a TCP one and an
    /// abstract unix one, and a socket of each kind, made before the box
    /// too, to connect to them from inside it once `socket` is refused.
    struct Listeners {
        _tcp: std::net::TcpListener,
        tcp_address: std::net::SocketAddr,
        _unix: std::os::unix::net::UnixListener,
        unix_address: std::os::unix::net::SocketAddr,
        tcp_socket: socket2::Socket,
        unix_socket: socket2::Socket,
    }

    impl Listeners {
        fn new() -> Self {
            let tcp = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let unix_address = abstract_address("out");
            let socket = |domain| socket2::Socket::new(domain, socket2::Type::STREAM, None);
            Self {
                tcp_address: tcp.local_addr().unwrap(),
                _tcp: tcp,
                _unix: std::os::unix::net::UnixListener::bind_addr(&unix_address).unwrap(),
                unix_address,
                tcp_socket: socket(socket2::Domain::IPV4).unwrap(),
                unix_socket: socket(socket2::Domain::UNIX).unwrap(),
            }
        }

        /// Connects the sockets made before the box to the listeners, and
        /// prints what each `connect` did.
        fn connect(&self) {
            use std::os::linux::net::SocketAddrExt as _;
            use std::os::unix::ffi::OsStrExt as _;
            println!(
                "tcp connect made before: {}",
                outcome(self.tcp_socket.connect(&self.tcp_address.into()))
            );
            // the abstract name, after its leading NUL
            let mut path = vec![0];
            path.extend_from_slice(self.unix_address.as_abstract_name().unwrap());
            let address = socket2::SockAddr::unix(std::ffi::OsStr::from_bytes(&path));
            println!(
                "connect abstract made before: {}",
                outcome(address.and_then(|address| self.unix_socket.connect(&address)))
            );
        }
    }

    /// Evaluates the project's module as `fgro eval` does: the pre-pass on a
    /// joined thread, the box from this one, and the engine on a thread
    /// started after it, which then tries a read outside the box.
    fn eval(project: &Path, outside: &Path) {
        let options = EvalOptions {
            entry: PathBuf::from("main.js"),
            cwd: project.to_owned(),
            export: None,
            driver: None,
            jit: false,
            snapshot: Some(StartupSnapshot {
                bytes: embedded::SNAPSHOT,
                residual_lazy_js: embedded::RESIDUAL_LAZY_JS,
            }),
            runtime: RuntimeInfo::new("0.0.0", None),
            timeout: Duration::from_secs(30),
            backstop: None,
            graph: false,
        };
        let sized = std::thread::Builder::new().stack_size(STACK_SIZE);
        let prepass = sized
            .spawn(move || gro_runtime::eval::prepass(options))
            .unwrap()
            .join()
            .unwrap()
            .unwrap();
        let Confinement::Confined(confined) = confine(project, Jit::Off) else {
            unreachable!("confine exits when it refuses")
        };
        let outside = outside.to_owned();
        let sized = std::thread::Builder::new().stack_size(STACK_SIZE);
        sized
            .spawn(move || {
                let engine = init_confined_engine(confined, &engine_options(false)).unwrap();
                let envelope = gro_runtime::eval::eval(engine, prepass);
                let value = envelope.result.map_err(|error| error.message).unwrap();
                println!("value: {value}");
                println!(
                    "read outside from the eval's thread: {}",
                    outcome(std::fs::read(&outside))
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
