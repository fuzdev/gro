//! `@fuzdev/gro/stdout.ts` and `@fuzdev/gro/stderr.ts`: what a write puts on
//! the stream, in what order beside the console, and what it resolves or
//! rejects with when the stream can't take it, which no differential case
//! can hold, since a case's stdout is its JSON and its stderr empty.

use std::io::Read as _;
use std::net::{TcpListener, TcpStream};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::support::{
    DEADLINE, FGRO, KillOnDrop, Scratch, drain, fgro_command, lines, spawn, wait_bounded,
    without_unboxed_note, wrapped_fgro_command,
};
use crate::{REFUSALS_FAIL, run_in, run_with, temp_fixtures};

/// Imports both streams, so a script below begins with it.
const IMPORTS: &str = "import { stdout } from '@fuzdev/gro/stdout.ts';\n\
                       import { stderr } from '@fuzdev/gro/stderr.ts';\n";

/// A project of `main.js`, [`IMPORTS`] then `body`.
fn project(name: &str, body: &str) -> Scratch {
    temp_fixtures(name, &[("main.js", &format!("{IMPORTS}{body}"))])
}

/// What `command`, a run, exited with, once it has, and what it printed on
/// stderr, its stdout set up by the caller.
fn exit_and_stderr(mut command: Command) -> (i32, String) {
    let mut child = command
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // the command holds what it was given for stdout until it's dropped
    drop(command);
    let stderr = drain(child.stderr.take().unwrap());
    let status = wait_bounded("the run", &mut child);
    let stderr = String::from_utf8(stderr.join().unwrap()).unwrap();
    (status.code().expect("the run exits"), stderr)
}

/// Each write's bytes are exactly its data, a string as UTF-8 and bytes as
/// they are, and the console's and both streams' writes land in program
/// order on a descriptor they share, an unawaited write's included: each is
/// written whole before its call returns.
#[test]
fn the_streams_and_the_console_keep_program_order() {
    let fixtures = project(
        "stdio_order",
        "await stdout.write('a\u{e9}');\n\
         console.log('b');\n\
         await stderr.write(new Uint8Array([0xff, 0x00, 0x0a]));\n\
         console.error('d');\n\
         stdout.write('e');\n\
         stderr.write('f');\n\
         console.log('g');\n\
         const written = [await stdout.write(''), await stderr.write(new Uint8Array(0))];\n\
         console.log(JSON.stringify(written));\n",
    );
    let (reader, writer) = std::io::pipe().unwrap();
    let mut command = fgro_command(&["run", "main.js"]);
    command
        .current_dir(&fixtures.0)
        .stdin(Stdio::null())
        .stdout(writer.try_clone().unwrap())
        .stderr(writer);
    let mut child = command.spawn().unwrap();
    drop(command);
    let output = drain(reader);
    let status = wait_bounded("the run", &mut child);
    assert_eq!(status.code(), Some(0));
    assert_eq!(
        output.join().unwrap(),
        b"a\xc3\xa9b\n\xff\x00\nd\nefg\n[true,true]\n"
    );
}

/// A write to a pipe whose reader has gone resolves `false`, and so does
/// every later write of data, as the kernel says each time, while an empty
/// one resolves `true`; the console's writes to it are dropped, and the run
/// goes on to its own exit code.
#[test]
fn a_gone_reader_resolves_false_and_the_run_goes_on() {
    let fixtures = project(
        "stdio_gone",
        "const written = [await stdout.write('a'), await stdout.write(new Uint8Array([1])), \
         await stdout.write('')];\n\
         console.log('dropped');\n\
         await stderr.write(JSON.stringify(written));\n",
    );
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    let mut command = fgro_command(&["run", "main.js"]);
    command.current_dir(&fixtures.0).stdout(writer);
    assert_eq!(
        exit_and_stderr(command),
        (0, "[false,false,true]".to_owned())
    );
}

/// A write the stream refuses for any other reason, a full disk, rejects
/// with `StdioError` `io_error`, its target the stream and its `cause` what
/// the OS said, and the next does too; the guard knows it.
#[test]
fn a_full_disk_rejects_with_io_error() {
    let fixtures = temp_fixtures(
        "stdio_full",
        &[(
            "main.js",
            &format!(
                "{IMPORTS}import {{ is_stdio_error }} from '@fuzdev/gro/stdio_error.ts';\n\
                 const failed = async (data) => stdout.write(data).then(\n\
                 \t(written) => written,\n\
                 \t(error) => [error.name, error.kind, error.target, error.message, \
                 is_stdio_error(error, 'io_error'), error.cause instanceof Error, \
                 error.cause.message.includes('No space left')],\n\
                 );\n\
                 const results = [await failed('x'), await failed(new Uint8Array([1])), await failed('')];\n\
                 await stderr.write(JSON.stringify(results));\n"
            ),
        )],
    );
    let full = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .unwrap();
    let mut command = fgro_command(&["run", "main.js"]);
    command.current_dir(&fixtures.0).stdout(full);
    let failure = r#"["StdioError","io_error","stdout","io_error: stdout",true,true,true]"#;
    assert_eq!(
        exit_and_stderr(command),
        (0, format!("[{failure},{failure},true]"))
    );
}

/// A write that failed partway leaves nothing behind for a later write to
/// carry out: with stdout a file the process may grow only to 512 bytes, a
/// write of 600 fails once the limit stops it, and once the file is
/// emptied, the next write puts only its own bytes there. std's line buffer
/// would have kept the 88 the kernel refused and written them first.
#[test]
fn a_failed_write_leaves_nothing_for_the_next() {
    let fixtures = temp_fixtures(
        "stdio_tail",
        &[(
            "main.js",
            &format!(
                "{IMPORTS}import {{ fs }} from '@fuzdev/gro/fs.ts';\n\
                 const first = await stdout.write('a'.repeat(600)).then(String, (error) => error.kind);\n\
                 await stderr.write('ready\\n');\n\
                 while (!(await fs.project.exists('go'))) await new Promise((resolve) => setTimeout(resolve, 10));\n\
                 const second = await stdout.write('b\\n');\n\
                 await stderr.write(`${{JSON.stringify([first, second])}}\\n`);\n"
            ),
        )],
    );
    let out = fixtures.0.join("out");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&out)
        .unwrap();
    // a write past the limit is `EFBIG`, not the signal that would end the
    // process, once the signal is ignored, which `exec` keeps
    let mut wrapper = Command::new("sh");
    wrapper.args([
        "-c",
        "trap '' XFSZ; exec prlimit --fsize=512:512 \"$@\"",
        "sh",
    ]);
    let mut command = wrapped_fgro_command(wrapper, &["run", "main.js"]);
    command
        .current_dir(&fixtures.0)
        .stdin(Stdio::null())
        .stdout(file)
        .stderr(Stdio::piped());
    // killed if the test fails before `go`, which the run would wait for forever
    let mut child = KillOnDrop(command.spawn().unwrap());
    drop(command);
    let stderr = lines(child.stderr.take().unwrap());
    assert_eq!(stderr.recv_timeout(DEADLINE).unwrap(), "ready\n");
    assert_eq!(std::fs::read(&out).unwrap(), [b'a'; 512]);
    std::fs::OpenOptions::new()
        .write(true)
        .open(&out)
        .unwrap()
        .set_len(0)
        .unwrap();
    std::fs::write(fixtures.0.join("go"), "").unwrap();
    assert_eq!(
        stderr.recv_timeout(DEADLINE).unwrap(),
        "[\"io_error\",true]\n"
    );
    assert_eq!(wait_bounded("the run", &mut child).code(), Some(0));
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "b\n");
}

/// A run started with its stdout closed writes to `/dev/null`, which std
/// opens there as the process starts, so a write and the console's both
/// succeed.
#[test]
fn a_closed_stdout_writes_nowhere() {
    let fixtures = project(
        "stdio_closed",
        "const written = await stdout.write('x');\n\
         console.log('y');\n\
         await stderr.write(JSON.stringify(written));\n",
    );
    let mut wrapper = Command::new("sh");
    wrapper.args(["-c", "exec \"$@\" >&-", "sh"]);
    let mut command = wrapped_fgro_command(wrapper, &["run", "main.js"]);
    command.current_dir(&fixtures.0);
    assert_eq!(exit_and_stderr(command), (0, "true".to_owned()));
}

/// `is_terminal` is each stream's own: on a pseudo-terminal through
/// util-linux's `script`, `true` for both, and `false` for the one a shell
/// redirection inside the terminal sends to a file.
#[test]
fn is_terminal_is_each_stream_s() {
    let fixtures = project(
        "stdio_terminal",
        "console.log(JSON.stringify([stdout.is_terminal, stderr.is_terminal]));\n",
    );
    let on_terminal = |redirect: &str| {
        let command = format!("'{FGRO}' run main.js {redirect}");
        let mut script = Command::new("script");
        script
            .args(["-qec", &command, "/dev/null"])
            .current_dir(&fixtures.0);
        let output = spawn(script, b"");
        assert!(output.status.success(), "{output:?}");
        String::from_utf8(without_unboxed_note(output.stdout)).unwrap()
    };
    assert_eq!(on_terminal(""), "[true,true]\r\n");
    assert_eq!(on_terminal("2>/dev/null"), "[true,false]\r\n");
    let file = fixtures.0.join("out");
    assert_eq!(on_terminal(&format!(">'{}'", file.display())), "");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "[false,true]\n");
}

/// The CPU time, in clock ticks, the process `pid` has spent, its threads'
/// together, from `/proc`.
fn cpu_ticks(pid: u32) -> u64 {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    // the fields after the command's name, which can hold spaces: the
    // state, then `utime` and `stime` as the 12th and 13th
    let fields: Vec<&str> = stat[stat.rfind(')').unwrap() + 1..]
        .split_whitespace()
        .collect();
    fields[11].parse::<u64>().unwrap() + fields[12].parse::<u64>().unwrap()
}

/// A stdout something else made non-blocking, a socket here, gets every
/// byte of a write larger than it holds, and the run waits for its reader
/// asleep, not spinning: a second of a full socket costs the run a sliver
/// of a second of CPU, where spinning costs the whole second.
#[test]
fn a_nonblocking_stdout_gets_every_byte_without_spinning() {
    const SIZE: usize = 4 << 20;
    let fixtures = project(
        "stdio_nonblocking",
        &format!(
            "await stderr.write('ready\\n');\n\
             const written = await stdout.write(new Uint8Array({SIZE}).fill(120));\n\
             await stderr.write(`${{written}}\\n`);\n"
        ),
    );
    let (mut reader, writer) = UnixStream::pair().unwrap();
    writer.set_nonblocking(true).unwrap();
    let mut command = fgro_command(&["run", "main.js"]);
    command
        .current_dir(&fixtures.0)
        .stdin(Stdio::null())
        .stdout(OwnedFd::from(writer))
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    drop(command);
    let stderr = lines(child.stderr.take().unwrap());
    assert_eq!(stderr.recv_timeout(DEADLINE).unwrap(), "ready\n");
    // the socket holds far less than the write, so it's full well before
    std::thread::sleep(Duration::from_millis(300));
    let before = cpu_ticks(child.id());
    std::thread::sleep(Duration::from_secs(1));
    let spent = cpu_ticks(child.id()) - before;
    let mut read = Vec::new();
    reader.read_to_end(&mut read).unwrap();
    assert_eq!(wait_bounded("the run", &mut child).code(), Some(0));
    assert_eq!(stderr.recv_timeout(DEADLINE).unwrap(), "true\n");
    assert!(
        read.len() == SIZE && read.iter().all(|&byte| byte == b'x'),
        "{} bytes",
        read.len()
    );
    assert!(spent < 30, "{spent} ticks of CPU in a second of waiting");
}

/// A package's module is refused each stream, the message naming the grant
/// and the package, while the project imports both in the same run; `-A`
/// grants a package the project's.
#[test]
fn a_package_is_refused_the_streams_unless_the_run_is_unconfined() {
    let fixtures = temp_fixtures(
        "stdio_package",
        &[
            ("package.json", r#"{"name": "app", "type": "module"}"#),
            (
                "node_modules/dep/package.json",
                r#"{"name": "dep", "type": "module", "exports": "./index.js"}"#,
            ),
            (
                "node_modules/dep/index.js",
                "export const load = (name) => import(`@fuzdev/gro/${name}.ts`);\n",
            ),
            (
                "main.js",
                &format!(
                    "{IMPORTS}import {{ load }} from 'dep';\n\
                     for (const [name, handle] of [['stdout', stdout], ['stderr', stderr]]) {{\n\
                     \tconst loaded = await load(name).catch((error) => error.message);\n\
                     \tconsole.log(typeof loaded === 'string' ? loaded : loaded[name] === handle);\n\
                     }}\n"
                ),
            ),
        ],
    );
    let run = run_in(&fixtures, "main.js");
    assert_eq!(run.code, 1, "{}", run.stderr);
    let refusal = |name: &str| {
        format!(
            "cannot import '@fuzdev/gro/{name}.ts' from file://{}/node_modules/dep/index.js: it \
             needs the grant '{name}', which package 'dep' isn't granted: only the project's own \
             modules are granted host modules\n",
            fixtures.0.display()
        )
    };
    assert_eq!(
        run.stdout,
        format!("{}{}", refusal("stdout"), refusal("stderr"))
    );
    assert_eq!(
        run.stderr,
        format!(
            "error: import() refused: {}error: import() refused: {}{REFUSALS_FAIL}",
            refusal("stdout"),
            refusal("stderr")
        )
    );
    let run = run_with(&fixtures, &["-A"], "main.js", &[]);
    assert_eq!(
        (run.code, run.stdout.as_str(), run.stderr.as_str()),
        (0, "true\ntrue\n", "")
    );
}

/// A project whose `main.js` writes `a` to stdout, says `ready` on stderr,
/// waits for a file `go`, then runs `body`; and the run started with its
/// stdout a TCP socket whose peer, once `a` has arrived there unread,
/// closes, which resets the connection, so the run's first write after
/// `go` meets `ECONNRESET`, and any later one `EPIPE`: its exit code and
/// what it printed on stderr after `ready`.
fn after_a_reset(name: &str, body: &str) -> (i32, String) {
    let fixtures = temp_fixtures(
        name,
        &[(
            "main.js",
            &format!(
                "{IMPORTS}import {{ fs }} from '@fuzdev/gro/fs.ts';\n\
                 await stdout.write('a');\n\
                 await stderr.write('ready\\n');\n\
                 while (!(await fs.project.exists('go'))) await new Promise((resolve) => setTimeout(resolve, 10));\n\
                 {body}"
            ),
        )],
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let ports = (
        client.local_addr().unwrap().port(),
        client.peer_addr().unwrap().port(),
    );
    let (peer, _) = listener.accept().unwrap();
    let mut command = fgro_command(&["run", "main.js"]);
    command
        .current_dir(&fixtures.0)
        .stdin(Stdio::null())
        .stdout(OwnedFd::from(client))
        .stderr(Stdio::piped());
    let mut child = KillOnDrop(command.spawn().unwrap());
    drop(command);
    let stderr = lines(child.stderr.take().unwrap());
    assert_eq!(stderr.recv_timeout(DEADLINE).unwrap(), "ready\n");
    // `a` is in the peer's buffer, unread, so closing it resets the connection
    assert_eq!(peer.peek(&mut [0; 1]).unwrap(), 1);
    drop(peer);
    // the reset reaches the run's end as the kernel processes it, which can
    // trail the close; once it has, the connection is no longer established
    let start = std::time::Instant::now();
    while established(ports) {
        assert!(
            start.elapsed() < DEADLINE,
            "the reset never reached {ports:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    std::fs::write(fixtures.0.join("go"), "").unwrap();
    let code = wait_bounded("the run", &mut child)
        .code()
        .expect("the run exits");
    (code, stderr.iter().collect())
}

/// Whether the IPv4 TCP connection from the local port to the remote one
/// of `ports` is established, from `/proc/net/tcp`, read without touching
/// the socket, so its pending error stays for the run's write to meet.
fn established((local, remote): (u16, u16)) -> bool {
    let table = std::fs::read_to_string("/proc/net/tcp").unwrap();
    let (local, remote) = (format!(":{local:04X}"), format!(":{remote:04X}"));
    table.lines().skip(1).any(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        fields[1].ends_with(&local) && fields[2].ends_with(&remote) && fields[3] == "01"
    })
}

/// A write to a socket its peer reset (`ECONNRESET`) resolves `false`, as
/// a broken pipe's does, and so does the next, which meets `EPIPE`; a
/// console write that meets the reset is dropped, and the run goes on to
/// its own exit code. The run's box refuses an internet socket on stdio
/// before anything runs (`run_boxed.rs`), so this runs where it can't go
/// up.
#[test]
#[cfg_attr(
    feature = "snapshot",
    ignore = "the run's box refuses an internet socket on stdio"
)]
fn a_reset_socket_resolves_false_and_the_console_drops_its_write() {
    assert_eq!(
        after_a_reset(
            "stdio_reset_write",
            "const written = [await stdout.write('b'), await stdout.write('c')];\n\
             await stderr.write(`${JSON.stringify(written)}\\n`);\n"
        ),
        (0, "[false,false]\n".to_owned())
    );
    assert_eq!(
        after_a_reset(
            "stdio_reset_console",
            "console.log('b');\n\
             await stderr.write(`${await stdout.write('c')}\\n`);\n"
        ),
        (0, "false\n".to_owned())
    );
}

/// A stdout open only for reading, which every write refuses with `EBADF`,
/// rejects a write with `StdioError` `io_error`, and fails a console write,
/// as any failure but a gone reader does, where std's own stdout would
/// have dropped it.
#[test]
fn a_stdout_open_for_reading_fails_each_write() {
    let fixtures = project(
        "stdio_read_only",
        "const failed = await stdout.write('x').then(String, (error) => error.kind);\n\
         await stderr.write(`${failed}\\n`);\n\
         console.log('y');\n",
    );
    let read_only = std::fs::File::open("/dev/null").unwrap();
    let mut command = fgro_command(&["run", "main.js"]);
    command.current_dir(&fixtures.0).stdout(read_only);
    let (code, stderr) = exit_and_stderr(command);
    assert_eq!(code, 1, "{stderr}");
    assert!(
        stderr.starts_with(
            "io_error\nerror: Uncaught (in promise) Error: failed to write to stdout: Bad file \
             descriptor (os error 9)\n"
        ),
        "{stderr}"
    );
}
