//! The run's seccomp denylist in the kernel: each case runs this binary
//! again, the child, which installs the denylist's filter for one network
//! form from its main thread, its only one, as a box installs it, then makes
//! calls through std and prints what each answered; the parent checks the
//! child's output. The interpreter in the unit tests pins every number's
//! answer; this shows the kernel takes the assembled program and answers as
//! the interpreter reads it.
//!
//! The binary runs without libtest's harness, whose threads would stay
//! outside the filter, and installs nothing in the parent.
//!
//! Two cases name a socket's protocol, which std can't: the child runs
//! `python3` under its filter for them, and fails them where there's none.

#![expect(clippy::unwrap_used, reason = "a test's setup and its failures panic")]

fn main() {
    #[cfg(target_arch = "x86_64")]
    match std::env::args().nth(1).as_deref() {
        Some("child") => denylist::child(&std::env::args().nth(2).unwrap_or_default()),
        _ => denylist::parent(),
    }
    #[cfg(not(target_arch = "x86_64"))]
    println!("skipped: the run's denylist is x86_64's");
}

#[cfg(target_arch = "x86_64")]
mod denylist {
    use std::fmt::Write as _;
    use std::io;
    use std::os::unix::net::{UnixDatagram, UnixStream};
    use std::path::Path;
    use std::process::Command;

    use gro_runtime::confine::NetForm;
    use gro_runtime::confine::seccomp::denylist::denylist_filter;

    /// How a call through std ended: `ok`, or the error number's name.
    fn answer<T>(result: io::Result<T>) -> String {
        match result {
            Ok(_) => "ok".to_owned(),
            Err(error) => match error.raw_os_error() {
                Some(libc::EPERM) => "EPERM".to_owned(),
                Some(libc::ENOSYS) => "ENOSYS".to_owned(),
                _ => format!("{error}"),
            },
        }
    }

    /// What python3, run under the filter, answers for an `AF_INET` socket
    /// of the type `kind` and the protocol `protocol`: `ok`, or the error
    /// number's name.
    fn socket_by_python(kind: i32, protocol: i32) -> String {
        let script = format!(
            "import errno, socket\n\
             try:\n    socket.socket(socket.AF_INET, {kind}, {protocol}).close()\n    print('ok')\n\
             except OSError as error:\n    print(errno.errorcode[error.errno])"
        );
        match Command::new("python3").args(["-I", "-c", &script]).output() {
            Ok(output) => String::from_utf8_lossy(&output.stdout).trim().to_owned(),
            Err(error) => format!("python3 failed to start: {error}"),
        }
    }

    /// What the child prints in each form: the internet sockets made in the
    /// run's own network namespace, and refused where it has none.
    pub fn parent() {
        let exe = std::env::current_exe().unwrap();
        for (form, inet) in [("namespace", "ok"), ("inet_refused", "EPERM")] {
            let expected = format!(
                "thread: ok\n\
                 program: exit 3\n\
                 metadata: ok\n\
                 unix stream pair: ok\n\
                 unix datagram pair: EPERM\n\
                 unix connect: EPERM\n\
                 tcp listener: {inet}\n\
                 udp socket: {inet}\n\
                 tcp socket by protocol: {inet}\n\
                 ping socket: EPERM\n\
                 chown: EPERM\n\
                 lchown: EPERM\n\
                 fchown: EPERM\n\
                 chroot: ENOSYS\n"
            );
            let output = Command::new(&exe).args(["child", form]).output().unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                output.status.success() && stdout == expected,
                "{form}: {}\nstdout:\n{stdout}\nexpected:\n{expected}\nstderr: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
            println!("test {form} ... ok");
        }
    }

    /// Installs the denylist for the form `form` and reports what each call
    /// answers. Every case is one the kernel would answer otherwise without
    /// the filter: a unix socket's connect `ENOENT`, `chown` to no change
    /// `ok`, `chroot` `EPERM` to an unprivileged process.
    pub fn child(form: &str) {
        let net = if form == "namespace" {
            NetForm::Namespace
        } else {
            assert_eq!(form, "inet_refused", "no network form");
            NetForm::InetRefused
        };
        let file = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("denylist_{form}"));
        std::fs::write(&file, "").unwrap();
        let filter = denylist_filter(net).unwrap();
        seccompiler::apply_filter(&filter).unwrap();
        let mut out = String::new();
        let mut line = |name: &str, answer: String| writeln!(out, "{name}: {answer}").unwrap();
        // glibc starts a thread by `clone3`, refused `ENOSYS`, then by
        // `clone`, whose flags the rule reads
        let thread = std::thread::Builder::new().spawn(|| 7);
        line(
            "thread",
            answer(thread.map(|thread| assert_eq!(thread.join().unwrap(), 7))),
        );
        let status = Command::new("/bin/sh").args(["-c", "exit 3"]).status();
        line(
            "program",
            match status {
                Ok(status) => format!("exit {}", status.code().unwrap_or(-1)),
                Err(error) => answer::<()>(Err(error)),
            },
        );
        line("metadata", answer(std::fs::metadata(&file)));
        line("unix stream pair", answer(UnixStream::pair()));
        line("unix datagram pair", answer(UnixDatagram::pair()));
        line(
            "unix connect",
            answer(UnixStream::connect("/nonexistent/denylist.sock")),
        );
        line(
            "tcp listener",
            answer(std::net::TcpListener::bind("127.0.0.1:0")),
        );
        line(
            "udp socket",
            answer(std::net::UdpSocket::bind("127.0.0.1:0")),
        );
        // a protocol named, then one the filter refuses before the kernel
        // sees it: ICMP's ping socket, built into the kernel, so no module
        // loads whatever the filter answers. std names no protocol, and this
        // crate makes no raw call, so python3 makes them, under the filter
        line("tcp socket by protocol", socket_by_python(1, 6));
        line("ping socket", socket_by_python(2, 1));

        line("chown", answer(std::os::unix::fs::chown(&file, None, None)));
        line(
            "lchown",
            answer(std::os::unix::fs::lchown(&file, None, None)),
        );
        let opened = std::fs::File::open(&file).unwrap();
        line(
            "fchown",
            answer(std::os::unix::fs::fchown(&opened, None, None)),
        );
        line("chroot", answer(std::os::unix::fs::chroot("/")));
        drop(opened);
        std::fs::remove_file(&file).unwrap();
        print!("{out}");
    }
}
