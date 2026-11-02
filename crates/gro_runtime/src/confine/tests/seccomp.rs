//! Tests for the seccomp tables and filters: the census lists, a BPF
//! interpreter, and the decision table.

use crate::confine::Jit;
use crate::confine::seccomp::{
    ALLOWED_CALLS, KEPT_ACCESS_MODE, KEPT_FCNTLS, KEPT_IOCTLS, KEPT_PRCTLS, KEPT_SOCKETPAIR,
    KILLED_CALLS, MAPPING_CALLS, seccomp_filters,
};

/// The filters compile for this architecture; installing them is the
/// confined-process tests' (a test process has threads of its own).
#[cfg(target_os = "linux")]
#[test]
fn the_seccomp_filters_compile() {
    for jit in [Jit::Off, Jit::On] {
        let [killing, allowing] = seccomp_filters(jit).unwrap();
        assert!(!killing.is_empty());
        assert!(!allowing.is_empty());
    }
}

/// The calls the allowlist allows whatever their arguments, listed apart
/// from [`ALLOWED_CALLS`], so a call added or dropped there fails
/// the tests: the calls the runtime was seen to make after the box goes
/// up, then the named siblings.
#[cfg(target_os = "linux")]
fn allowed_calls() -> Vec<libc::c_long> {
    let mut expected = vec![
        libc::SYS_brk,
        libc::SYS_clock_gettime,
        libc::SYS_clock_nanosleep,
        libc::SYS_close,
        libc::SYS_epoll_create1,
        libc::SYS_epoll_ctl,
        libc::SYS_eventfd2,
        libc::SYS_exit,
        libc::SYS_exit_group,
        libc::SYS_futex,
        libc::SYS_getpid,
        libc::SYS_gettid,
        libc::SYS_lseek,
        libc::SYS_madvise,
        libc::SYS_mremap,
        libc::SYS_munmap,
        libc::SYS_openat,
        libc::SYS_pkey_alloc,
        libc::SYS_read,
        libc::SYS_rseq,
        libc::SYS_rt_sigaction,
        libc::SYS_rt_sigprocmask,
        libc::SYS_sched_getaffinity,
        libc::SYS_sched_getparam,
        libc::SYS_sched_getscheduler,
        libc::SYS_sched_yield,
        libc::SYS_set_robust_list,
        libc::SYS_sigaltstack,
        libc::SYS_statx,
        libc::SYS_uname,
        libc::SYS_write,
        libc::SYS_getrandom,
        libc::SYS_epoll_pwait,
        libc::SYS_epoll_pwait2,
        libc::SYS_fstat,
        libc::SYS_newfstatat,
        libc::SYS_rt_sigreturn,
        libc::SYS_restart_syscall,
        libc::SYS_tgkill,
        libc::SYS_clock_getres,
        libc::SYS_gettimeofday,
        libc::SYS_nanosleep,
    ];
    #[cfg(target_arch = "x86_64")]
    expected.extend([libc::SYS_readlink, libc::SYS_epoll_wait]);
    #[cfg(target_arch = "aarch64")]
    expected.push(libc::SYS_readlinkat);
    expected
}

/// The calls the allowlist allows only by their arguments, the mapping
/// calls jitless.
#[cfg(target_os = "linux")]
const RULED_CALLS: [libc::c_long; 9] = [
    libc::SYS_prctl,
    libc::SYS_fcntl,
    libc::SYS_ioctl,
    libc::SYS_socketpair,
    libc::SYS_faccessat2,
    libc::SYS_clone,
    libc::SYS_mmap,
    libc::SYS_mprotect,
    libc::SYS_pkey_mprotect,
];

/// A sample of the calls the allowlist doesn't allow: every call the
/// box once refused by name (the socket calls, the metadata calls, the
/// watches, the System V and POSIX queue calls, the calls that read or
/// set a process's limits, and the ones that set its priority or
/// scheduling), then the namespace,
/// key, filter, process, and filesystem calls nothing in the runtime
/// makes.
#[cfg(target_os = "linux")]
fn unlisted_calls() -> Vec<libc::c_long> {
    vec![
        libc::SYS_socket,
        libc::SYS_connect,
        libc::SYS_bind,
        libc::SYS_listen,
        libc::SYS_accept4,
        libc::SYS_sendto,
        libc::SYS_sendmsg,
        libc::SYS_faccessat,
        libc::SYS_fchmod,
        libc::SYS_fchmodat,
        // `fchmodat2`, `setxattrat`, `removexattrat`, and `file_setattr`,
        // by their numbers in the table x86_64 and aarch64 share from 424
        452,
        463,
        466,
        469,
        libc::SYS_fchown,
        libc::SYS_fchownat,
        libc::SYS_utimensat,
        libc::SYS_setxattr,
        libc::SYS_lsetxattr,
        libc::SYS_fsetxattr,
        libc::SYS_removexattr,
        libc::SYS_lremovexattr,
        libc::SYS_fremovexattr,
        libc::SYS_inotify_add_watch,
        libc::SYS_fanotify_init,
        libc::SYS_mq_open,
        libc::SYS_mq_unlink,
        libc::SYS_shmget,
        libc::SYS_shmat,
        libc::SYS_shmctl,
        libc::SYS_msgget,
        libc::SYS_msgsnd,
        libc::SYS_msgrcv,
        libc::SYS_msgctl,
        libc::SYS_semget,
        libc::SYS_semop,
        libc::SYS_semtimedop,
        libc::SYS_semctl,
        libc::SYS_prlimit64,
        libc::SYS_getrlimit,
        libc::SYS_setrlimit,
        libc::SYS_setpriority,
        libc::SYS_ioprio_set,
        libc::SYS_sched_setaffinity,
        libc::SYS_sched_setscheduler,
        libc::SYS_sched_setparam,
        libc::SYS_sched_setattr,
        libc::SYS_clone3,
        libc::SYS_unshare,
        libc::SYS_setns,
        libc::SYS_keyctl,
        libc::SYS_add_key,
        libc::SYS_request_key,
        libc::SYS_seccomp,
        libc::SYS_landlock_create_ruleset,
        libc::SYS_landlock_restrict_self,
        libc::SYS_kill,
        libc::SYS_tkill,
        libc::SYS_pidfd_open,
        libc::SYS_pidfd_send_signal,
        libc::SYS_process_madvise,
        libc::SYS_personality,
        libc::SYS_setuid,
        libc::SYS_capset,
        libc::SYS_flock,
        libc::SYS_mount,
        libc::SYS_pivot_root,
        libc::SYS_chdir,
        libc::SYS_getcwd,
        libc::SYS_getdents64,
        libc::SYS_pipe2,
        libc::SYS_memfd_create,
        libc::SYS_mknodat,
        libc::SYS_mkdirat,
        libc::SYS_unlinkat,
        libc::SYS_renameat2,
        libc::SYS_linkat,
        libc::SYS_symlinkat,
        libc::SYS_ftruncate,
        libc::SYS_fallocate,
        libc::SYS_name_to_handle_at,
        libc::SYS_open_by_handle_at,
        libc::SYS_pread64,
        libc::SYS_readv,
        libc::SYS_writev,
        libc::SYS_set_tid_address,
        libc::SYS_close_range,
        libc::SYS_clock_settime,
        libc::SYS_reboot,
        #[cfg(target_arch = "x86_64")]
        libc::SYS_chmod,
        #[cfg(target_arch = "x86_64")]
        libc::SYS_chown,
        #[cfg(target_arch = "x86_64")]
        libc::SYS_lchown,
        #[cfg(target_arch = "x86_64")]
        libc::SYS_utime,
        #[cfg(target_arch = "x86_64")]
        libc::SYS_utimes,
        #[cfg(target_arch = "x86_64")]
        libc::SYS_futimesat,
        #[cfg(target_arch = "x86_64")]
        libc::SYS_open,
        #[cfg(target_arch = "x86_64")]
        libc::SYS_fork,
        #[cfg(target_arch = "x86_64")]
        libc::SYS_vfork,
        #[cfg(target_arch = "x86_64")]
        libc::SYS_readlinkat,
    ]
}

/// The allowlist is the list above, plus the calls allowed by their
/// arguments, each with the values pinned here; none is a killed call,
/// and the sample of unlisted calls is unlisted.
#[cfg(target_os = "linux")]
#[test]
fn the_allowlist_is_the_census() {
    let mut allowed = ALLOWED_CALLS.to_vec();
    let mut expected = allowed_calls();
    allowed.sort_unstable();
    expected.sort_unstable();
    assert_eq!(allowed, expected);
    allowed.dedup();
    assert_eq!(allowed.len(), ALLOWED_CALLS.len(), "a call listed twice");
    for call in KILLED_CALLS.into_iter().chain(RULED_CALLS) {
        assert!(!allowed.contains(&call), "{call}");
    }
    for call in unlisted_calls() {
        assert!(!allowed.contains(&call), "{call}");
        assert!(!RULED_CALLS.contains(&call), "{call}");
    }
    assert_eq!(KEPT_PRCTLS, [libc::PR_SET_NAME, libc::PR_SET_VMA]);
    assert_eq!(
        KEPT_FCNTLS,
        [
            libc::F_GETFD,
            libc::F_SETFD,
            libc::F_GETFL,
            libc::F_SETFL,
            libc::F_DUPFD_CLOEXEC,
        ]
    );
    assert_eq!(KEPT_SOCKETPAIR, [libc::AF_UNIX, libc::SOCK_STREAM]);
    assert_eq!(KEPT_ACCESS_MODE, libc::F_OK);
    assert_eq!(
        MAPPING_CALLS,
        [libc::SYS_mmap, libc::SYS_mprotect, libc::SYS_pkey_mprotect]
    );
}

/// The `ioctl` requests kept are the ones std, tokio, and the console
/// make, and the allowlist compares the request with each; the
/// ones that set a file's attributes aren't kept.
#[cfg(target_os = "linux")]
#[test]
fn the_kept_ioctls_set_nothing() {
    assert_eq!(
        KEPT_IOCTLS,
        [
            libc::FIONBIO,
            libc::FIOCLEX,
            libc::FIONCLEX,
            libc::FIONREAD,
            libc::TCGETS,
            libc::TIOCGWINSZ,
        ]
    );
    for refused in [
        libc::FS_IOC_SETFLAGS,
        libc::FS_IOC32_SETFLAGS,
        libc::FS_IOC_SETVERSION,
        libc::FS_IOC32_SETVERSION,
        0x401c_5820,
    ] {
        assert!(!KEPT_IOCTLS.contains(&refused), "{refused:#x}");
    }
    let [_, allowing] = seccomp_filters(Jit::Off).unwrap();
    for request in KEPT_IOCTLS {
        assert!(
            allowing
                .iter()
                .any(|instruction| u64::from(instruction.k) == request),
            "{request:#x}"
        );
    }
}

/// The `seccomp_data` architecture value of the kernel's ABI this test
/// runs on: `AUDIT_ARCH_X86_64` or `AUDIT_ARCH_AARCH64`, the
/// architectures the box is built for.
#[cfg(target_os = "linux")]
fn audit_arch() -> u32 {
    match std::env::consts::ARCH {
        "x86_64" => 0xc000_003e,
        "aarch64" => 0xc000_00b7,
        other => panic!("no seccomp_data architecture value for {other}"),
    }
}

/// What the kernel would decide for the call `nr` with `args`, from the
/// architecture `arch`, under the compiled `filter`: a classic BPF
/// interpreter over the instructions `seccompiler` emits, run on the
/// `seccomp_data` the kernel hands a filter (`nr` and `arch`, 32 bits
/// each, the instruction pointer, then the six arguments, 64 bits each,
/// little-endian on both architectures), so a table can pin the policy
/// as compiled. Returns the filter's return value, its action and data.
/// An instruction it doesn't know panics, never passes.
#[cfg(target_os = "linux")]
fn decide(filter: &[seccompiler::sock_filter], arch: u32, nr: libc::c_long, args: [u64; 6]) -> u32 {
    const LOAD: u32 = libc::BPF_LD | libc::BPF_W | libc::BPF_ABS;
    const AND: u32 = libc::BPF_ALU | libc::BPF_AND | libc::BPF_K;
    const JUMP: u32 = libc::BPF_JMP | libc::BPF_JA;
    const EQUAL: u32 = libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K;
    const ABOVE: u32 = libc::BPF_JMP | libc::BPF_JGT | libc::BPF_K;
    const AT_LEAST: u32 = libc::BPF_JMP | libc::BPF_JGE | libc::BPF_K;
    const RETURN: u32 = libc::BPF_RET | libc::BPF_K;
    let mut data = [0_u8; 64];
    data[..4].copy_from_slice(&i32::try_from(nr).unwrap().to_le_bytes());
    data[4..8].copy_from_slice(&arch.to_le_bytes());
    for (slot, arg) in data[16..].as_chunks_mut::<8>().0.iter_mut().zip(args) {
        slot.copy_from_slice(&arg.to_le_bytes());
    }
    let (mut pc, mut accumulator) = (0, 0_u32);
    loop {
        let instruction = &filter[pc];
        pc += 1;
        let k = instruction.k;
        let taken = |taken: bool| {
            usize::from(if taken {
                instruction.jt
            } else {
                instruction.jf
            })
        };
        match u32::from(instruction.code) {
            LOAD => {
                let at = usize::try_from(k).unwrap();
                accumulator = u32::from_le_bytes(data[at..at + 4].try_into().unwrap());
            }
            AND => accumulator &= k,
            JUMP => pc += usize::try_from(k).unwrap(),
            EQUAL => pc += taken(accumulator == k),
            ABOVE => pc += taken(accumulator > k),
            AT_LEAST => pc += taken(accumulator >= k),
            RETURN => return k,
            code => panic!("an instruction the evaluator doesn't know: {code:#x}"),
        }
    }
}

/// The filters decide each call as the policy says, read as the kernel
/// reads them, for a box with the JIT off and one with it on. The
/// allowlist allows every listed call, and the calls allowed by their
/// arguments only with the kept values, an int's upper half, which the
/// kernel drops, changing nothing; jitless, the mapping calls only with
/// `PROT_EXEC` clear in `prot`, and with the JIT, whatever their
/// arguments; every other call, an x32 call among them, fails with
/// `ENOSYS`. The killing filter allows all of those and kills on its own
/// calls, and both kill a call from another architecture.
#[cfg(target_os = "linux")]
#[test]
fn the_filters_decide_by_the_allowlist() {
    const ALLOW: u32 = libc::SECCOMP_RET_ALLOW;
    const ENOSYS: u32 = libc::SECCOMP_RET_ERRNO | libc::ENOSYS.unsigned_abs();
    const KILL: u32 = libc::SECCOMP_RET_KILL_PROCESS;
    let arch = audit_arch();
    let high = 0xdead_0000_0000_0000_u64;
    let int = |value: libc::c_int| u64::from(value.unsigned_abs());
    let unix = int(libc::AF_UNIX);
    let [stream, datagram, seqpacket, cloexec, nonblock] = [
        libc::SOCK_STREAM,
        libc::SOCK_DGRAM,
        libc::SOCK_SEQPACKET,
        libc::SOCK_CLOEXEC,
        libc::SOCK_NONBLOCK,
    ]
    .map(int);
    // glibc's flags for a thread, and for `fork` and `posix_spawn`
    let thread = int(libc::CLONE_VM
        | libc::CLONE_FS
        | libc::CLONE_FILES
        | libc::CLONE_SIGHAND
        | libc::CLONE_THREAD
        | libc::CLONE_SYSVSEM
        | libc::CLONE_SETTLS
        | libc::CLONE_PARENT_SETTID
        | libc::CLONE_CHILD_CLEARTID);
    // `AT_FDCWD` as glibc passes it, the int's bits
    let at_cwd = u64::from(libc::AT_FDCWD.cast_unsigned());
    let [f_ok, r_ok, w_ok, x_ok, at_eaccess] = [
        libc::F_OK,
        libc::R_OK,
        libc::W_OK,
        libc::X_OK,
        libc::AT_EACCESS,
    ]
    .map(int);
    let sigchld = int(libc::SIGCHLD);
    let fork = int(libc::CLONE_CHILD_SETTID | libc::CLONE_CHILD_CLEARTID) | sigchld;
    let spawn = int(libc::CLONE_VM | libc::CLONE_VFORK) | sigchld;
    let mut cases = vec![
        // a thread, also with an upper half the kernel drops from
        // `clone`'s flags; a fork, a spawn, and a namespace refused
        (libc::SYS_clone, [thread, 0, 0, 0, 0, 0], ALLOW),
        (
            libc::SYS_clone,
            [int(libc::CLONE_THREAD), 0, 0, 0, 0, 0],
            ALLOW,
        ),
        (libc::SYS_clone, [high | thread, 0, 0, 0, 0, 0], ALLOW),
        (libc::SYS_clone, [fork, 0, 0, 0, 0, 0], ENOSYS),
        (libc::SYS_clone, [spawn, 0, 0, 0, 0, 0], ENOSYS),
        (libc::SYS_clone, [0, 0, 0, 0, 0, 0], ENOSYS),
        (
            libc::SYS_clone,
            [int(libc::CLONE_NEWUSER | libc::CLONE_THREAD), 0, 0, 0, 0, 0],
            ENOSYS,
        ),
        // the kept prctls, also with an upper half, and others refused
        (
            libc::SYS_prctl,
            [int(libc::PR_SET_NAME), 0, 0, 0, 0, 0],
            ALLOW,
        ),
        (
            libc::SYS_prctl,
            [high | int(libc::PR_SET_NAME), 0, 0, 0, 0, 0],
            ALLOW,
        ),
        (
            libc::SYS_prctl,
            [int(libc::PR_SET_VMA), 0, 0, 0, 0, 0],
            ALLOW,
        ),
        // the kept fcntls, also with an upper half, and others refused
        (libc::SYS_fcntl, [3, int(libc::F_GETFD), 0, 0, 0, 0], ALLOW),
        (
            libc::SYS_fcntl,
            [3, high | int(libc::F_SETFL), 0, 0, 0, 0],
            ALLOW,
        ),
        // the kept ioctls, also with an upper half, and others refused
        (
            libc::SYS_ioctl,
            [2, high | libc::FIONBIO, 0, 0, 0, 0],
            ALLOW,
        ),
        (libc::SYS_ioctl, [2, libc::TCGETS, 0, 0, 0, 0], ALLOW),
        (
            libc::SYS_ioctl,
            [2, high | libc::FS_IOC_SETFLAGS, 0, 0, 0, 0],
            ENOSYS,
        ),
        (
            libc::SYS_ioctl,
            [2, libc::FS_IOC_GETFLAGS, 0, 0, 0, 0],
            ENOSYS,
        ),
        (libc::SYS_ioctl, [2, libc::TIOCSTI, 0, 0, 0, 0], ENOSYS),
        (libc::SYS_ioctl, [2, 0, 0, 0, 0, 0], ENOSYS),
        // the kept socketpair, with its flags, and others refused
        (libc::SYS_socketpair, [unix, stream, 0, 0, 0, 0], ALLOW),
        (
            libc::SYS_socketpair,
            [unix, stream | cloexec | nonblock, 0, 0, 0, 0],
            ALLOW,
        ),
        (libc::SYS_socketpair, [unix, datagram, 0, 0, 0, 0], ENOSYS),
        (libc::SYS_socketpair, [unix, seqpacket, 0, 0, 0, 0], ENOSYS),
        (
            libc::SYS_socketpair,
            [int(libc::AF_INET), stream, 0, 0, 0, 0],
            ENOSYS,
        ),
        // the kept access mode, as glibc's `realpath` asks it and also
        // with an upper half, and the modes asking for access refused
        (
            libc::SYS_faccessat2,
            [at_cwd, 0, f_ok, at_eaccess, 0, 0],
            ALLOW,
        ),
        (
            libc::SYS_faccessat2,
            [at_cwd, 0, high | f_ok, 0, 0, 0],
            ALLOW,
        ),
        (libc::SYS_faccessat2, [at_cwd, 0, r_ok, 0, 0, 0], ENOSYS),
        (
            libc::SYS_faccessat2,
            [at_cwd, 0, w_ok, at_eaccess, 0, 0],
            ENOSYS,
        ),
        (libc::SYS_faccessat2, [at_cwd, 0, x_ok, 0, 0, 0], ENOSYS),
        (
            libc::SYS_faccessat2,
            [at_cwd, 0, r_ok | w_ok | x_ok, 0, 0, 0],
            ENOSYS,
        ),
        // `faccessat`, glibc's fallback, refused even for `F_OK`
        (libc::SYS_faccessat, [at_cwd, 0, f_ok, 0, 0, 0], ENOSYS),
    ];
    for namespace in [
        libc::CLONE_NEWNS,
        libc::CLONE_NEWCGROUP,
        libc::CLONE_NEWUTS,
        libc::CLONE_NEWIPC,
        libc::CLONE_NEWUSER,
        libc::CLONE_NEWPID,
        libc::CLONE_NEWNET,
    ] {
        cases.push((
            libc::SYS_clone,
            [thread | int(namespace), 0, 0, 0, 0, 0],
            ENOSYS,
        ));
    }
    for option in [
        libc::PR_SET_DUMPABLE,
        libc::PR_SET_NO_NEW_PRIVS,
        libc::PR_SET_SECCOMP,
        libc::PR_SET_PTRACER,
        libc::PR_SET_PDEATHSIG,
        libc::PR_CAP_AMBIENT,
        libc::PR_SET_MM,
        0,
    ] {
        cases.push((libc::SYS_prctl, [int(option), 0, 0, 0, 0, 0], ENOSYS));
    }
    for command in KEPT_FCNTLS {
        cases.push((libc::SYS_fcntl, [3, int(command), 0, 0, 0, 0], ALLOW));
    }
    for command in [
        libc::F_DUPFD,
        libc::F_SETLK,
        libc::F_SETLKW,
        libc::F_GETLK,
        libc::F_OFD_SETLK,
        libc::F_SETLEASE,
        libc::F_GETLEASE,
        libc::F_NOTIFY,
        libc::F_SETOWN,
        libc::F_SETPIPE_SZ,
        libc::F_ADD_SEALS,
    ] {
        cases.push((libc::SYS_fcntl, [3, int(command), 0, 0, 0, 0], ENOSYS));
        cases.push((
            libc::SYS_fcntl,
            [3, high | int(command), 0, 0, 0, 0],
            ENOSYS,
        ));
    }
    for request in KEPT_IOCTLS {
        cases.push((libc::SYS_ioctl, [2, request, 0, 0, 0, 0], ALLOW));
    }
    for call in allowed_calls() {
        cases.push((call, [0; 6], ALLOW));
    }
    for call in unlisted_calls() {
        cases.push((call, [0; 6], ENOSYS));
    }
    // an x32 call, the x86_64 call number with bit 30 set, matches no
    // number the filters compare
    #[cfg(target_arch = "x86_64")]
    for call in allowed_calls().into_iter().chain(RULED_CALLS) {
        cases.push((call | 0x4000_0000, [0; 6], ENOSYS));
    }
    // the mapping calls' `prot`, the third argument, and whether a
    // jitless box refuses it: `PROT_EXEC` clear passes, also with an
    // upper half the kernel's flag test never reads as `PROT_EXEC`, with
    // its bit shifted into that half, and with the bit set in every
    // other argument; `PROT_EXEC` set is refused, beside any other flag
    // and under any upper half
    let [read, write, exec, grows_down] = [
        libc::PROT_READ,
        libc::PROT_WRITE,
        libc::PROT_EXEC,
        libc::PROT_GROWSDOWN,
    ]
    .map(int);
    let exec_elsewhere = [exec, exec, read, exec, exec, exec];
    let mappings = [
        ([0, 4096, 0, 0, 0, 0], false),
        ([0, 4096, read, 0, 0, 0], false),
        ([0, 4096, read | write, 0, 0, 0], false),
        ([0, 4096, high | read | write, 0, 0, 0], false),
        ([0, 4096, exec << 32, 0, 0, 0], false),
        ([0, 4096, grows_down | read, 0, 0, 0], false),
        (exec_elsewhere, false),
        ([0, 4096, exec, 0, 0, 0], true),
        ([0, 4096, read | exec, 0, 0, 0], true),
        ([0, 4096, read | write | exec, 0, 0, 0], true),
        ([0, 4096, high | exec, 0, 0, 0], true),
        ([0, 4096, grows_down | read | exec, 0, 0, 0], true),
        ([0, 4096, 0xffff_ffff, 0, 0, 0], true),
        ([0, 4096, u64::MAX, 0, 0, 0], true),
    ];
    for jit in [Jit::Off, Jit::On] {
        let [killing, allowing] = seccomp_filters(jit).unwrap();
        let mut cases = cases.clone();
        // listed apart from `MAPPING_CALLS`, so a call dropped there
        // fails here too
        for call in [libc::SYS_mmap, libc::SYS_mprotect, libc::SYS_pkey_mprotect] {
            for (args, exec_set) in mappings {
                let expected = if exec_set && jit == Jit::Off {
                    ENOSYS
                } else {
                    ALLOW
                };
                cases.push((call, args, expected));
            }
        }
        for (call, args, expected) in cases {
            assert_eq!(
                decide(&allowing, arch, call, args),
                expected,
                "{jit:?} {call} {args:x?}"
            );
            assert_eq!(
                decide(&killing, arch, call, args),
                ALLOW,
                "{jit:?} {call} {args:x?}"
            );
        }
        for call in KILLED_CALLS {
            assert_eq!(decide(&killing, arch, call, [0; 6]), KILL, "{call}");
            assert_eq!(decide(&allowing, arch, call, [0; 6]), ENOSYS, "{call}");
            // by its x32 number, past the killing filter, the allowlist
            // still refuses it
            #[cfg(target_arch = "x86_64")]
            {
                assert_eq!(decide(&killing, arch, call | 0x4000_0000, [0; 6]), ALLOW);
                assert_eq!(decide(&allowing, arch, call | 0x4000_0000, [0; 6]), ENOSYS);
            }
        }
        for filter in [&killing, &allowing] {
            assert_eq!(decide(filter, arch ^ 1, libc::SYS_read, [0; 6]), KILL);
        }
    }
}
