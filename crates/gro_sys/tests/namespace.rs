//! `enter_user_net_namespace` and `loopback_up` from inside a child process:
//! each case runs this binary again, the child, which enters the namespaces
//! or is refused them and reports what it then sees; the parent reads its
//! output. A user namespace takes a process of one thread, which libtest's
//! harness isn't, and the namespaces are the whole process's, so they're
//! never entered in this one.
//!
//! They need unprivileged user namespaces, which a kernel restricting them
//! (`kernel.apparmor_restrict_unprivileged_userns`,
//! `kernel.unprivileged_userns_clone`) refuses: a case then fails, naming
//! the setting, never skips. The filtered cases start the child under a
//! seccomp filter refusing `unshare`, which `python3` installs before it
//! runs the child, so this crate's tests make no `unsafe` call of their own.

#![expect(clippy::unwrap_used, reason = "a test's setup and its failures panic")]
#![expect(clippy::panic, reason = "a test fails by panicking")]

mod support;

use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::process::Command;

use gro_sys::namespace::{self, NamespaceError, Refusal};

fn main() {
    support::dispatch(parent, child);
}

/// Runs each case in a child and checks what it reports.
fn parent() {
    let exe = std::env::current_exe().unwrap();
    let uid = id_field("Uid:");
    let gid = id_field("Gid:");
    // a listener outside the namespaces, on the parent's loopback, which a
    // child inside must not reach
    let outside = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = outside.local_addr().unwrap().port();
    for (case, expected) in [
        (
            "enters",
            format!(
                "uid_map: {uid} {uid} 1\ngid_map: {gid} {gid} 1\nsetgroups: deny\nids: {uid} \
                 {gid}\ninterfaces: lo\nconnects to itself before: no\nup: ok\nconnects to \
                 itself after: yes\nreaches outside: no\nits child sees: lo\n"
            ),
        ),
        (
            "none_left",
            format!("entered: ok\nrefused: NoneLeft, 28\nuid_map: {uid} {uid} 1\ninterfaces: lo\n"),
        ),
        (
            "threads",
            "threads: 2\nuid_map: 0 0 4294967295\ninterfaces unchanged: true\n".to_owned(),
        ),
        // a process marked not dumpable can't write its maps: the order the
        // module doc gives, pinned
        ("dump_then_enter", "stranded: Setgroups, 13\n".to_owned()),
        (
            "filtered_eperm",
            "refused: Filtered, 1\nuid_map: 0 0 4294967295\n".to_owned(),
        ),
        (
            "filtered_enosys",
            "refused: Filtered, 38\nuid_map: 0 0 4294967295\n".to_owned(),
        ),
    ] {
        let mut command = match case {
            "filtered_eperm" => filtered(&exe, libc::EPERM),
            "filtered_enosys" => filtered(&exe, libc::ENOSYS),
            _ => Command::new(&exe),
        };
        command
            .args(["child", case])
            .env("OUTSIDE_PORT", port.to_string());
        support::run_case(case, command, &expected);
    }
}

/// Runs `case` in this process, the child.
fn child(case: &str) {
    match case {
        // the maps written, the ids kept, only loopback there, down (its
        // address unreachable) until it's brought up, then reachable from
        // inside, and nothing outside reachable
        "enters" => {
            let entered = support::or_chain(namespace::enter_user_net_namespace());
            println!("uid_map: {}", first_map_line("uid_map"));
            println!("gid_map: {}", first_map_line("gid_map"));
            println!(
                "setgroups: {}",
                std::fs::read_to_string("/proc/self/setgroups")
                    .unwrap()
                    .trim()
            );
            println!("ids: {} {}", id_field("Uid:"), id_field("Gid:"));
            println!("interfaces: {}", interfaces().join(" "));
            println!("connects to itself before: {}", connects_to_itself());
            support::or_chain(entered.loopback_up());
            println!("up: ok");
            println!("connects to itself after: {}", connects_to_itself());
            let port: u16 = std::env::var("OUTSIDE_PORT").unwrap().parse().unwrap();
            println!(
                "reaches outside: {}",
                if TcpStream::connect((Ipv4Addr::LOCALHOST, port)).is_ok() {
                    "yes"
                } else {
                    "no"
                }
            );
            let net_dev = Command::new("cat")
                .arg("/proc/self/net/dev")
                .output()
                .unwrap()
                .stdout;
            println!(
                "its child sees: {}",
                interface_names(&String::from_utf8(net_dev).unwrap()).join(" ")
            );
        }
        // once nested user namespaces are refused, the next is refused with
        // nothing changed: the first namespace's map stands
        "none_left" => {
            let entered = support::or_chain(namespace::enter_user_net_namespace());
            println!("entered: ok");
            entered.refuse_nested_user_namespaces().unwrap();
            print_refused(namespace::enter_user_net_namespace());
            println!("uid_map: {}", first_map_line("uid_map"));
            println!("interfaces: {}", interfaces().join(" "));
        }
        // a second thread is refused by the kernel, with nothing changed:
        // the initial namespace's map and every interface stand
        "threads" => {
            let before = interfaces();
            let (ready, wait) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                ready.send(()).unwrap();
                std::thread::park();
            });
            wait.recv().unwrap();
            match namespace::enter_user_net_namespace() {
                Err(NamespaceError::Threads { count }) => println!("threads: {count}"),
                other => panic!("not refused for its threads: {other:?}"),
            }
            println!("uid_map: {}", first_map_line("uid_map"));
            println!("interfaces unchanged: {}", interfaces() == before);
        }
        "dump_then_enter" => {
            gro_sys::set_not_dumpable().unwrap();
            match namespace::enter_user_net_namespace() {
                Err(NamespaceError::Stranded { step, source }) => {
                    println!("stranded: {step:?}, {}", source.raw_os_error().unwrap());
                }
                other => panic!("not stranded: {other:?}"),
            }
        }
        // under a filter refusing `unshare`, refused with nothing changed
        "filtered_eperm" | "filtered_enosys" => {
            print_refused(namespace::enter_user_net_namespace());
            println!("uid_map: {}", first_map_line("uid_map"));
        }
        _ => unreachable!("no case {case}"),
    }
}

/// `exe` run by `python3`, which first installs a seccomp filter refusing
/// `unshare` with `errno` and allowing every other call, as a container
/// runtime's profile does, then replaces itself with `exe`.
fn filtered(exe: &std::path::Path, errno: libc::c_int) -> Command {
    // `struct sock_filter`s: load the call number, `unshare`'s returns the
    // error, any other is allowed
    const SCRIPT: &str = r#"
import ctypes, os, struct, sys
nr, errno = int(sys.argv[1]), int(sys.argv[2])
code = struct.pack("HBBI", 0x20, 0, 0, 0)
code += struct.pack("HBBI", 0x15, 0, 1, nr)
code += struct.pack("HBBI", 0x06, 0, 0, 0x00050000 | errno)
code += struct.pack("HBBI", 0x06, 0, 0, 0x7fff0000)
buffer = ctypes.create_string_buffer(code)
class Program(ctypes.Structure):
    _fields_ = [("len", ctypes.c_ushort), ("filter", ctypes.c_void_p)]
program = Program(4, ctypes.cast(buffer, ctypes.c_void_p))
libc = ctypes.CDLL(None, use_errno=True)
ulong = ctypes.c_ulong
assert libc.prctl(38, ulong(1), ulong(0), ulong(0), ulong(0)) == 0
assert libc.prctl(22, ulong(2), ctypes.byref(program), ulong(0), ulong(0)) == 0
os.execv(sys.argv[3], sys.argv[3:])
"#;
    let mut command = Command::new("python3");
    command
        .args(["-I", "-c", SCRIPT])
        .arg(libc::SYS_unshare.to_string())
        .arg(errno.to_string())
        .arg(exe);
    command
}

/// Prints a refusal's cause and error code; anything else fails.
fn print_refused(result: Result<namespace::UserNetNamespace, NamespaceError>) {
    match result {
        Err(NamespaceError::Refused(refused)) => {
            let code = std::error::Error::source(&refused)
                .and_then(|source| source.downcast_ref::<std::io::Error>())
                .and_then(std::io::Error::raw_os_error)
                .unwrap();
            let cause: Refusal = refused.cause();
            println!("refused: {cause:?}, {code}");
        }
        other => panic!("not refused: {other:?}"),
    }
}

/// The process's effective id from `/proc/self/status`'s `field` line
/// (`Uid:` or `Gid:`), its second number.
fn id_field(field: &str) -> String {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let line = status
        .lines()
        .find_map(|line| line.strip_prefix(field))
        .unwrap();
    line.split_whitespace().nth(1).unwrap().to_owned()
}

/// The first line of `/proc/self/<map>`, its fields one space apart.
fn first_map_line(map: &str) -> String {
    let text = std::fs::read_to_string(format!("/proc/self/{map}")).unwrap();
    text.lines()
        .next()
        .unwrap()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The network interfaces the process's namespace holds.
fn interfaces() -> Vec<String> {
    interface_names(&std::fs::read_to_string("/proc/self/net/dev").unwrap())
}

/// The interfaces `/proc/net/dev`'s text lists, past its two header lines.
fn interface_names(net_dev: &str) -> Vec<String> {
    net_dev
        .lines()
        .skip(2)
        .map(|line| line.split(':').next().unwrap().trim().to_owned())
        .collect()
}

/// Whether a listener on loopback's address takes a connection from this
/// process: `"no"` while loopback is down, as its address is bound but
/// unreachable (`ENETUNREACH`).
fn connects_to_itself() -> &'static str {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    if TcpStream::connect(listener.local_addr().unwrap()).is_ok() {
        listener.accept().unwrap();
        "yes"
    } else {
        "no"
    }
}
