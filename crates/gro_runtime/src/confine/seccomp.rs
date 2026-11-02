//! The box's seccomp filters: the call tables and the compiled pair of
//! programs, a killing tripwire and the allowlist.

use super::{ConfineError, Jit};

/// The calls that kill the process, by their own filter, installed before
/// the allowlist.
///
/// None is made by any part of the runtime, so reaching one means code
/// escaped the engine. The allowlist doesn't allow them either; this filter
/// is the tripwire that ends the process rather than let it carry on past
/// `ENOSYS`.
pub(super) const KILLED_CALLS: [libc::c_long; 16] = [
    libc::SYS_execve,
    libc::SYS_execveat,
    libc::SYS_ptrace,
    libc::SYS_process_vm_readv,
    libc::SYS_process_vm_writev,
    libc::SYS_bpf,
    libc::SYS_perf_event_open,
    libc::SYS_userfaultfd,
    libc::SYS_kexec_load,
    libc::SYS_kexec_file_load,
    libc::SYS_io_uring_setup,
    libc::SYS_io_uring_enter,
    libc::SYS_io_uring_register,
    libc::SYS_init_module,
    libc::SYS_finit_module,
    libc::SYS_delete_module,
];

/// The calls the allowlist filter allows whatever their arguments.
///
/// They are the calls the runtime was seen to make after the box goes up,
/// and the named siblings a host may make in their place. Every call
/// missing from it and from the calls allowed by their arguments
/// ([`KEPT_PRCTLS`], [`KEPT_FCNTLS`], [`KEPT_IOCTLS`], [`KEPT_SOCKETPAIR`],
/// [`KEPT_ACCESS_MODE`], a `clone` that starts a thread, setting no flag in
/// [`CLONE_NAMESPACES`], and the [`MAPPING_CALLS`]) fails with `ENOSYS`.
///
/// `sched_getaffinity`, `sched_getparam`, and `sched_getscheduler` take any
/// process, since glibc names the caller by its thread id, not 0: for
/// another process they tell whether it exists and how it's scheduled, not
/// its files or memory. Built for `x86_64` and `aarch64`; the `aarch64` list
/// is written from its call table and unverified on a host.
pub(super) const ALLOWED_CALLS: &[libc::c_long] = &[
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
    // aarch64 has only `readlinkat`, which glibc's `readlink` makes there
    #[cfg(target_arch = "x86_64")]
    libc::SYS_readlink,
    #[cfg(target_arch = "aarch64")]
    libc::SYS_readlinkat,
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
    // V8's entropy, whose failure aborts the process (`engine`): the vDSO's
    // on glibc 2.41 and Linux 6.11 and later, a call elsewhere
    libc::SYS_getrandom,
    // the event loop's wait, made only when it blocks: which one depends on
    // the architecture and the libc (aarch64 has no `epoll_wait`)
    #[cfg(target_arch = "x86_64")]
    libc::SYS_epoll_wait,
    libc::SYS_epoll_pwait,
    libc::SYS_epoll_pwait2,
    // std's fallback when `statx` fails
    libc::SYS_fstat,
    libc::SYS_newfstatat,
    // the return from a handled signal (`--jit`'s WebAssembly trap handler)
    // and the restart of a call it interrupted
    libc::SYS_rt_sigreturn,
    libc::SYS_restart_syscall,
    // `abort()`'s signal to its own thread; Landlock's signal scope keeps
    // it inside the box
    libc::SYS_tgkill,
    // the clock calls' fallbacks when the vDSO lacks one, and `nanosleep`,
    // which other libcs make for a sleep
    libc::SYS_clock_getres,
    libc::SYS_gettimeofday,
    libc::SYS_nanosleep,
];

/// The calls that set a mapping's protection, by their third argument,
/// `prot`, on `x86_64` and `aarch64` alike.
///
/// A box for the JIT ([`Jit::On`]), whose compiled code needs executable
/// memory, allows them whatever their arguments; a jitless one only with
/// `PROT_EXEC` clear in `prot`, so no mapping becomes executable after the
/// box goes up. `mremap` keeps a mapping's protection, so it stays allowed:
/// it can move, copy, or grow an executable mapping made before the box,
/// over its file's own bytes, but never over bytes the process wrote.
pub(super) const MAPPING_CALLS: [libc::c_long; 3] =
    [libc::SYS_mmap, libc::SYS_mprotect, libc::SYS_pkey_mprotect];

/// The `prctl` options the box keeps, by the first argument: naming a
/// thread, and naming an anonymous mapping (glibc's and V8's). Every other
/// fails with `ENOSYS`, the ones that change what the process may do
/// (`PR_SET_NO_NEW_PRIVS`, `PR_SET_SECCOMP`, the capability options) or
/// who may reach it (`PR_SET_DUMPABLE`, `PR_SET_PTRACER`) among them.
pub(super) const KEPT_PRCTLS: [libc::c_int; 2] = [libc::PR_SET_NAME, libc::PR_SET_VMA];

/// The `fcntl` commands the box keeps, by the second argument: reading and
/// setting the descriptor and status flags, and duplicating a descriptor
/// close-on-exec, the ones std and tokio use. Every other fails with
/// `ENOSYS`: the advisory locks, which would hold off another process
/// locking the same file inside the box, a lease, which stalls another
/// process's open for writing and tells the holder it was tried, directory
/// change notices, and setting a descriptor's owner among them.
pub(super) const KEPT_FCNTLS: [libc::c_int; 5] = [
    libc::F_GETFD,
    libc::F_SETFD,
    libc::F_GETFL,
    libc::F_SETFL,
    libc::F_DUPFD_CLOEXEC,
];

/// The `ioctl` requests the box keeps: setting and clearing non-blocking
/// and close-on-exec, the bytes waiting, and reading a terminal's settings
/// and size. Every other request fails with `ENOSYS`, the ones that change
/// a file's attribute flags, generation, extended attributes, or verity, or
/// a filesystem's own (a btrfs subvolume), from a handle opened only to
/// read among them, which Landlock gates only on devices.
pub(super) const KEPT_IOCTLS: [libc::Ioctl; 6] = [
    libc::FIONBIO,
    libc::FIOCLEX,
    libc::FIONCLEX,
    libc::FIONREAD,
    libc::TCGETS,
    libc::TIOCGWINSZ,
];

/// The one `socketpair` the box keeps, by domain and type: a unix stream
/// pair, which tokio uses, connected only to itself.
///
/// Every other fails with `ENOSYS`, a datagram pair above all, which can
/// send to any socket file by its path.
pub(super) const KEPT_SOCKETPAIR: [libc::c_int; 2] = [libc::AF_UNIX, libc::SOCK_STREAM];

/// The one `faccessat2` mode the box keeps, by the third argument: `F_OK`,
/// whether the path exists, which `statx` tells already and Landlock gates for
/// neither. glibc's `realpath`, behind `std::fs::canonicalize` and so the
/// resolver's every lookup, asks it of a directory that a `.` or `..` component
/// or a trailing slash follows, as a link's target `/x/a/b/../../pkg` or
/// `/x/pkg/` has. Every other mode, which asks whether the process may read,
/// write, or execute the path, fails with `ENOSYS`.
///
/// `faccessat`, glibc's fallback when `faccessat2` fails with `ENOSYS`,
/// isn't allowed: every kernel the box runs on (Landlock ABI 6, Linux 6.12)
/// has `faccessat2`, which arrived in Linux 5.8.
pub(super) const KEPT_ACCESS_MODE: libc::c_int = libc::F_OK;

/// An `ioctl` request as the filter compares it, whichever type the libc
/// declares requests as: glibc's `c_ulong`, as is.
#[cfg(not(target_env = "musl"))]
const fn ioctl_request(request: libc::Ioctl) -> u64 {
    request
}

/// An `ioctl` request as the filter compares it, whichever type the libc
/// declares requests as: musl's `c_int`, by its bits, as the kernel reads
/// the `unsigned int` it is.
#[cfg(target_env = "musl")]
fn ioctl_request(request: libc::Ioctl) -> u64 {
    u64::from(request.cast_unsigned())
}

/// The bits of a socket type that name it, below the `SOCK_NONBLOCK` and
/// `SOCK_CLOEXEC` flags.
const SOCKET_TYPE_MASK: u64 = 0xf;

/// The namespace flags `clone` accepts, which the box's one kept `clone`
/// must not set: the flags under the mask of these and `CLONE_THREAD`, the
/// first argument on `x86_64` and `aarch64`, must be `CLONE_THREAD` alone, so a
/// thread starts, and no process is forked and no namespace made.
/// `CLONE_NEWTIME` isn't among them: `clone` reads its flags' low byte as
/// the exit signal, and takes that flag only from `clone3`.
///
/// `clone3` isn't allowed: its flags sit in memory, out of a filter's reach,
/// and its `ENOSYS` makes glibc start the thread with `clone` instead.
const CLONE_NAMESPACES: libc::c_int = libc::CLONE_NEWNS
    | libc::CLONE_NEWCGROUP
    | libc::CLONE_NEWUTS
    | libc::CLONE_NEWIPC
    | libc::CLONE_NEWUSER
    | libc::CLONE_NEWPID
    | libc::CLONE_NEWNET;

/// The box's two seccomp filters, for an engine whose JIT is `jit`,
/// compiled for this architecture, in the order they are installed: the
/// calls that kill the process, then the allowlist, which fails every call
/// it doesn't allow with `ENOSYS`.
pub(super) fn seccomp_filters(jit: Jit) -> Result<[seccompiler::BpfProgram; 2], ConfineError> {
    use std::collections::BTreeMap;

    use seccompiler::{
        SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompFilter, SeccompRule,
    };
    let arch = seccompiler::TargetArch::try_from(std::env::consts::ARCH).map_err(|_| {
        ConfineError::SeccompArch {
            arch: std::env::consts::ARCH,
        }
    })?;
    let killed: BTreeMap<i64, Vec<SeccompRule>> = KILLED_CALLS
        .into_iter()
        .map(|call| (call, Vec::new()))
        .collect();
    // a call with rules is allowed when any rule matches, and a rule matches
    // when all its conditions hold; an int argument is compared at 32 bits,
    // the half the kernel reads
    let condition = |argument, length, op, value| {
        SeccompCondition::new(argument, length, op, value).map_err(ConfineError::SeccompFilter)
    };
    let rule = |conditions| SeccompRule::new(conditions).map_err(ConfineError::SeccompFilter);
    let each_value = |argument, values: &[u64]| {
        values
            .iter()
            .map(|value| {
                rule(vec![condition(
                    argument,
                    SeccompCmpArgLen::Dword,
                    SeccompCmpOp::Eq,
                    *value,
                )?])
            })
            .collect::<Result<Vec<_>, _>>()
    };
    // an int's bits as the kernel passes them, a negative one's included
    let int = |value: libc::c_int| u64::from(value.cast_unsigned());
    // `prctl`'s option, the first argument, and `fcntl`'s command, the
    // second, ints
    let prctls = each_value(0, &KEPT_PRCTLS.map(int))?;
    let fcntls = each_value(1, &KEPT_FCNTLS.map(int))?;
    // `ioctl`'s request, the second argument, an unsigned int
    let ioctls = each_value(1, &KEPT_IOCTLS.map(ioctl_request))?;
    // `faccessat2`'s mode, the third argument, an int
    let accesses = each_value(2, &[int(KEPT_ACCESS_MODE)])?;
    // `socketpair`'s domain, the first argument, and its type, the second,
    // its flags masked off
    let [kept_domain, kept_type] = KEPT_SOCKETPAIR.map(int);
    let pairs = vec![rule(vec![
        condition(0, SeccompCmpArgLen::Dword, SeccompCmpOp::Eq, kept_domain)?,
        condition(
            1,
            SeccompCmpArgLen::Dword,
            SeccompCmpOp::MaskedEq(SOCKET_TYPE_MASK),
            kept_type,
        )?,
    ])?];
    // `clone`'s flags, the first argument, an unsigned long, compared whole:
    // a thread, and no namespace
    let thread = int(libc::CLONE_THREAD);
    let threads = vec![rule(vec![condition(
        0,
        SeccompCmpArgLen::Qword,
        SeccompCmpOp::MaskedEq(thread | int(CLONE_NAMESPACES)),
        thread,
    )?])?];
    // the mapping calls' `prot`, the third argument, an int: any with the
    // JIT, and jitless only with `PROT_EXEC` clear
    let mapping = || match jit {
        Jit::On => Ok(Vec::new()),
        Jit::Off => Ok(vec![rule(vec![condition(
            2,
            SeccompCmpArgLen::Dword,
            SeccompCmpOp::MaskedEq(int(libc::PROT_EXEC)),
            0,
        )?])?]),
    };
    let mappings = MAPPING_CALLS
        .into_iter()
        .map(|call| Ok((call, mapping()?)))
        .collect::<Result<Vec<_>, ConfineError>>()?;
    let allowed: BTreeMap<i64, Vec<SeccompRule>> = ALLOWED_CALLS
        .iter()
        .map(|call| (*call, Vec::new()))
        .chain(mappings)
        .chain([
            (libc::SYS_prctl, prctls),
            (libc::SYS_fcntl, fcntls),
            (libc::SYS_ioctl, ioctls),
            (libc::SYS_socketpair, pairs),
            (libc::SYS_faccessat2, accesses),
            (libc::SYS_clone, threads),
        ])
        .collect();
    let compile = |rules, mismatch, matched| {
        SeccompFilter::new(rules, mismatch, matched, arch)
            .and_then(seccompiler::BpfProgram::try_from)
            .map_err(ConfineError::SeccompFilter)
    };
    Ok([
        compile(killed, SeccompAction::Allow, SeccompAction::KillProcess)?,
        compile(
            allowed,
            SeccompAction::Errno(libc::ENOSYS.unsigned_abs()),
            SeccompAction::Allow,
        )?,
    ])
}
