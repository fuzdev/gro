//! The run's seccomp denylist.
//!
//! A pinned `x86_64` table that classifies every call number, allowed or
//! refused with its family's reason, and the filter assembled from it
//! ([`denylist_filter`]) for each of the box's network forms ([`NetForm`]).
//!
//! It's a kernel boundary for the run's box, beside Landlock, against code
//! built to defeat the box (a project's tests, a dependency's genfile, a tool
//! the run spawns, an engine exploit). It is the only layer for:
//!
//! - unix sockets by path, which Landlock doesn't gate: the session bus,
//!   through which the user's service manager starts any program outside
//!   the box, a container daemon, an ssh or gpg agent;
//! - nested user namespaces, which would hand code capabilities over its own
//!   network stack;
//! - the keyrings, the user's session keyring among them;
//! - System V IPC and POSIX message queues, shared by key or name with the
//!   user's processes outside the box;
//! - changing a file's owner or group, or its extended attributes;
//! - every socket in the [`NetForm::InetRefused`] form, where Landlock holds
//!   only TCP's bind and connect.
//!
//! It is a second layer for another process's memory and descriptors
//! (beside Landlock's ptrace walls and the runtime's process being
//! non-dumpable), and defense in depth for the kernel's wider surface (BPF,
//! performance counters, `io_uring`, modules, an inet protocol's module, the
//! calls no command was seen to make), which sysctls hold in part on some
//! hosts. Refusing to set another process's limits guards availability,
//! which the threat model leaves outside.
//!
//! What defeats it is a family the table allows that reaches an asset, or a
//! kernel bug. The stated gaps: re-moding and retiming a file the user owns,
//! and setting its attribute flags by `ioctl` or `file_setattr`; another of the user's
//! processes' scheduling; and a path's metadata and link target, and an
//! inotify watch, which Landlock doesn't gate, so the run learns the names
//! of entries made, changed, and removed in any directory the user can
//! reach, one the box can't list among them. A call outside every family is
//! no way past it: every number from 0 to the highest classified is in the
//! table, the [`UNASSIGNED`] numbers included, and every number above it
//! fails with `ENOSYS`, so a call a newer kernel adds is refused until the
//! table classifies it, and so is an x32 call (`nr | 0x40000000`). A call
//! from another architecture kills the process. The kernel runs one call
//! without consulting any filter: `uretprobe`, which the table allows.
//!
//! Unlike eval's allowlist, which allows only what its census saw, the
//! denylist allows what tools ordinarily do (spawning, files, sockets that
//! stay inside the box's network) and refuses by family. A whole family is
//! refused [`Refusal::Absent`] (`ENOSYS`, as a kernel without the call
//! answers, so a libc falls back where it can: `clone3` to `clone`, `io_uring`
//! to epoll), and a call refused by its arguments [`Refusal::NotPermitted`]
//! (`EPERM`), as are the ownership and extended-attribute calls, whose `EPERM`
//! tools report as the refusal it is.
//!
//! The run's box installs it after Landlock
//! ([`RunPrepared::confine`](crate::confine::run::RunPrepared::confine)),
//! the box every command of Gro's and `fgro run` run in; `fgro run -A`, or one where the box can't go up, holds none
//! ([`Confinement::Unconfined`](crate::confine::Confinement::Unconfined)).
//! The table is `x86_64`'s, whose numbers no other architecture shares, so
//! the module builds only there.

use std::ops::RangeInclusive;

use seccompiler::BpfProgram;

use super::tree::{self, Action, ArgMatch, Decision, TreeError};
use super::{CLONE_NAMESPACES, SOCKET_TYPE_MASK, ioctl_request};
use crate::confine::NetForm;

/// `AUDIT_ARCH_X86_64`, the `seccomp_data` architecture of an `x86_64`
/// call: `EM_X86_64`, 64-bit, little-endian.
const AUDIT_ARCH_X86_64: u32 = 0xc000_003e;

/// The numbers no `x86_64` call has.
///
/// They run past `uretprobe`, the last of the architecture's own numbering,
/// to the numbering every architecture shares, which starts at 424. A
/// kernel answers them `ENOSYS` itself; the filter answers first.
pub const UNASSIGNED: RangeInclusive<u32> = 336..=423;

/// The `ioctl` requests refused: pushing bytes into a terminal's input
/// queue (`TIOCSTI`), which the shell reading it runs as typed, and the
/// Linux console's subcommands (`TIOCLINUX`), which include the same.
pub const REFUSED_IOCTLS: [libc::Ioctl; 2] = [libc::TIOCSTI, libc::TIOCLINUX];

/// The `personality` arguments allowed: `0xffffffff`, which only reads it,
/// and `PER_LINUX` (0), which sets no flag.
pub const KEPT_PERSONALITIES: [u32; 2] = [0xffff_ffff, 0];

/// The socket domains a run makes sockets of in its own network namespace
/// ([`NetForm::Namespace`]): IPv4 and IPv6.
///
/// There they reach only the namespace's loopback. Every other domain is
/// refused, `AF_UNIX` above all, whose connections to a socket file (an
/// ssh agent, a container daemon, the session bus) Landlock doesn't gate,
/// and `AF_NETLINK`, `AF_PACKET`, `AF_VSOCK`, and `AF_ALG` with it.
pub const INET_DOMAINS: [libc::c_int; 2] = [libc::AF_INET, libc::AF_INET6];

/// The internet sockets allowed in the run's own network namespace, by
/// type (its flags masked off) and protocol: TCP's stream and UDP's
/// datagram, each with its protocol named or left 0.
///
/// Every other type and protocol is refused: a raw socket, and a protocol
/// whose module `socket` would load into the kernel (SCTP, DCCP, and the
/// rest), and a type named with another type's protocol, which fails
/// anyway.
pub const KEPT_INET_SOCKETS: [(libc::c_int, libc::c_int); 2] = [
    (libc::SOCK_STREAM, libc::IPPROTO_TCP),
    (libc::SOCK_DGRAM, libc::IPPROTO_UDP),
];

/// The socket pairs allowed, by domain and type (its flags masked off).
///
/// A unix stream or sequenced-packet pair is connected only to itself,
/// which Node and its tools make for a child's pipes. A datagram pair is
/// refused: it can send to any datagram socket file by its path.
pub const KEPT_SOCKETPAIRS: [(libc::c_int, libc::c_int); 2] = [
    (libc::AF_UNIX, libc::SOCK_STREAM),
    (libc::AF_UNIX, libc::SOCK_SEQPACKET),
];

/// How the denylist answers a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The call runs, whatever its arguments.
    Allow,
    /// The call fails, whatever its arguments.
    Refuse(Refusal),
    /// The call runs or fails by its arguments, as the rule says.
    Ruled(ArgRule),
}

/// The error a refused call fails with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// `ENOSYS`, what a kernel without the call answers: a program sees the
    /// feature as missing, and a libc falls back where it has a fallback.
    Absent,
    /// `EPERM`: the call exists, and this process may not make it.
    NotPermitted,
}

impl Refusal {
    /// The error number.
    pub const fn errno(self) -> libc::c_int {
        match self {
            Self::Absent => libc::ENOSYS,
            Self::NotPermitted => libc::EPERM,
        }
    }
}

/// A call allowed or refused by its arguments, each compared by the low 32
/// bits the kernel reads; a refusal is [`Refusal::NotPermitted`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgRule {
    /// `clone`, refused when its flags set a namespace flag, so a fork or a
    /// thread starts and no namespace is made.
    Clone,
    /// `ioctl`, refused for the [`REFUSED_IOCTLS`].
    Ioctl,
    /// `personality`, allowed for the [`KEPT_PERSONALITIES`] only.
    Personality,
    /// `prlimit64`, allowed only on the calling process (pid 0), so a
    /// process sets and reads its own limits and no other's.
    Prlimit,
    /// `socket`, allowed for the [`INET_DOMAINS`] and the
    /// [`KEPT_INET_SOCKETS`] in the run's own network namespace, and refused
    /// whatever its arguments where the run has none
    /// ([`NetForm::InetRefused`]).
    Socket,
    /// `socketpair`, allowed for the [`KEPT_SOCKETPAIRS`] only.
    Socketpair,
}

/// A family of calls the table answers alike, for one reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Family {
    /// What the calls are.
    pub name: &'static str,
    /// Why they're answered so.
    pub reason: &'static str,
    /// How they're answered.
    pub verdict: Verdict,
    /// The calls.
    pub calls: &'static [Call],
}

/// A call, by its `x86_64` number and its name in the kernel's table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Call {
    /// The number.
    pub nr: u32,
    /// The name, as the kernel's `__NR_` constants spell it.
    pub name: &'static str,
}

const fn call(nr: u32, name: &'static str) -> Call {
    Call { nr, name }
}

/// The table: every `x86_64` call number but the [`UNASSIGNED`] ones, each
/// in one family.
pub static FAMILIES: &[Family] = &[
    // the only layer: the families nothing else in the box holds
    Family {
        name: "socket",
        reason: "the only layer for unix sockets by path, which Landlock doesn't gate: the \
                 session bus, through which the user's service manager starts any program \
                 outside the box, a container daemon, an ssh or gpg agent; and, in the \
                 `InetRefused` form, the only layer for every socket, Landlock holding only \
                 TCP's bind and connect. In the run's own network namespace, a TCP or UDP \
                 socket alone, so no other protocol's module is loaded into the kernel",
        verdict: Verdict::Ruled(ArgRule::Socket),
        calls: &[call(41, "socket")],
    },
    Family {
        name: "socketpair",
        reason: "the only layer for a unix datagram pair, which can send to any datagram socket \
                 file by its path; a connected unix stream or sequenced-packet pair, for a \
                 child's pipes, stays",
        verdict: Verdict::Ruled(ArgRule::Socketpair),
        calls: &[call(53, "socketpair")],
    },
    Family {
        name: "namespaces and mounts",
        reason: "the only layer for nested user namespaces, which would hand code capabilities \
                 over its own network stack (netfilter, traffic control, routing), and Landlock \
                 refuses mounts but not `unshare`: a new namespace or mount, and reading the \
                 mount table; `clone3` whole, since its flags sit in memory out of a filter's \
                 reach, so glibc falls back to `clone`, whose flags the `clone` rule reads",
        verdict: Verdict::Refuse(Refusal::Absent),
        calls: &[
            call(155, "pivot_root"),
            call(161, "chroot"),
            call(165, "mount"),
            call(166, "umount2"),
            call(272, "unshare"),
            call(308, "setns"),
            call(428, "open_tree"),
            call(429, "move_mount"),
            call(430, "fsopen"),
            call(431, "fsconfig"),
            call(432, "fsmount"),
            call(433, "fspick"),
            call(435, "clone3"),
            call(442, "mount_setattr"),
            call(457, "statmount"),
            call(458, "listmount"),
            call(467, "open_tree_attr"),
        ],
    },
    Family {
        name: "clone",
        reason: "a fork or a thread, never a new namespace, the namespaces' only layer with \
                 `unshare` and `clone3`",
        verdict: Verdict::Ruled(ArgRule::Clone),
        calls: &[call(56, "clone")],
    },
    Family {
        name: "keyrings",
        reason: "the only layer for the keyrings, the user's session keyring among them, whose \
                 keys a process of the same user may read",
        verdict: Verdict::Refuse(Refusal::Absent),
        calls: &[
            call(248, "add_key"),
            call(249, "request_key"),
            call(250, "keyctl"),
        ],
    },
    Family {
        name: "System V IPC and POSIX message queues",
        reason: "the only layer for shared memory, semaphores, and message queues named by a \
                 key or a name the user's other processes share, outside the box and out of \
                 Landlock's sight",
        verdict: Verdict::Refuse(Refusal::Absent),
        calls: &[
            call(29, "shmget"),
            call(30, "shmat"),
            call(31, "shmctl"),
            call(64, "semget"),
            call(65, "semop"),
            call(66, "semctl"),
            call(67, "shmdt"),
            call(68, "msgget"),
            call(69, "msgsnd"),
            call(70, "msgrcv"),
            call(71, "msgctl"),
            call(220, "semtimedop"),
            call(240, "mq_open"),
            call(241, "mq_unlink"),
            call(242, "mq_timedsend"),
            call(243, "mq_timedreceive"),
            call(244, "mq_notify"),
            call(245, "mq_getsetattr"),
        ],
    },
    Family {
        name: "ownership and extended attributes",
        reason: "the only layer for changing a file's owner or group, or setting or removing \
                 its extended attributes, on any file the user owns, which Landlock doesn't \
                 gate; refused `EPERM`, which tools report as the refusal it is",
        verdict: Verdict::Refuse(Refusal::NotPermitted),
        calls: &[
            call(92, "chown"),
            call(93, "fchown"),
            call(94, "lchown"),
            call(188, "setxattr"),
            call(189, "lsetxattr"),
            call(190, "fsetxattr"),
            call(197, "removexattr"),
            call(198, "lremovexattr"),
            call(199, "fremovexattr"),
            call(260, "fchownat"),
            call(463, "setxattrat"),
            call(466, "removexattrat"),
        ],
    },
    // a second layer, and defense in depth
    Family {
        name: "other processes",
        reason: "a second layer for another process's memory and descriptors, beside Landlock's \
                 ptrace walls, which refuse a process outside the box, and the runtime's \
                 process being non-dumpable: reading, writing, comparing, or reaping them, the \
                 runtime's own among them, whose memory and descriptors no program it runs is \
                 handed",
        verdict: Verdict::Refuse(Refusal::Absent),
        calls: &[
            call(101, "ptrace"),
            call(310, "process_vm_readv"),
            call(311, "process_vm_writev"),
            call(312, "kcmp"),
            call(438, "pidfd_getfd"),
            call(440, "process_madvise"),
            call(448, "process_mrelease"),
        ],
    },
    Family {
        name: "kernel surface",
        reason: "defense in depth for the kernel's wider attack surface, which sysctls hold in \
                 part on some hosts: BPF programs, performance counters, page faults handled in \
                 user space, `io_uring` (whose operations bypass seccomp; Node 24 and 26 set up \
                 a ring per event loop and fall back to epoll and the threadpool on `ENOSYS`), \
                 loading a kernel or a module, opening a file by a handle, which bypasses the \
                 path Landlock checks, and the local descriptor table",
        verdict: Verdict::Refuse(Refusal::Absent),
        calls: &[
            call(154, "modify_ldt"),
            call(175, "init_module"),
            call(176, "delete_module"),
            call(246, "kexec_load"),
            call(298, "perf_event_open"),
            call(303, "name_to_handle_at"),
            call(304, "open_by_handle_at"),
            call(313, "finit_module"),
            call(320, "kexec_file_load"),
            call(321, "bpf"),
            call(323, "userfaultfd"),
            call(425, "io_uring_setup"),
            call(426, "io_uring_enter"),
            call(427, "io_uring_register"),
        ],
    },
    Family {
        name: "uncalled",
        reason: "defense in depth: reachable kernel code that no command the box runs was seen to \
                 call (`gro check` and each of its tools, traced), whose callers (most of \
                 libaio's, libnuma) treat `ENOSYS` as absent and fall back: Linux asynchronous \
                 I/O, NUMA memory placement (`migrate_pages` and `move_pages` naming another \
                 process), remapping a file's pages, secret memory, splicing user pages into a \
                 pipe, fanotify, whose names a watch already gets from inotify, and the old \
                 filesystem-type query",
        verdict: Verdict::Refuse(Refusal::Absent),
        calls: &[
            call(139, "sysfs"),
            call(206, "io_setup"),
            call(207, "io_destroy"),
            call(208, "io_getevents"),
            call(209, "io_submit"),
            call(210, "io_cancel"),
            call(216, "remap_file_pages"),
            call(237, "mbind"),
            call(238, "set_mempolicy"),
            call(239, "get_mempolicy"),
            call(256, "migrate_pages"),
            call(278, "vmsplice"),
            call(279, "move_pages"),
            call(300, "fanotify_init"),
            call(301, "fanotify_mark"),
            call(333, "io_pgetevents"),
            call(447, "memfd_secret"),
            call(450, "set_mempolicy_home_node"),
        ],
    },
    Family {
        name: "system administration",
        reason: "the machine's own settings, which need a capability the box doesn't hold \
                 or reach past the run: rebooting, swap, process accounting, quotas, the \
                 kernel log, setting the clock, the host and domain names, port I/O, \
                 hanging up the terminal, and the removed `uselib` and `lookup_dcookie`",
        verdict: Verdict::Refuse(Refusal::Absent),
        calls: &[
            call(103, "syslog"),
            call(134, "uselib"),
            call(153, "vhangup"),
            call(159, "adjtimex"),
            call(163, "acct"),
            call(164, "settimeofday"),
            call(167, "swapon"),
            call(168, "swapoff"),
            call(169, "reboot"),
            call(170, "sethostname"),
            call(171, "setdomainname"),
            call(172, "iopl"),
            call(173, "ioperm"),
            call(179, "quotactl"),
            call(212, "lookup_dcookie"),
            call(227, "clock_settime"),
            call(305, "clock_adjtime"),
            call(443, "quotactl_fd"),
        ],
    },
    Family {
        name: "unimplemented",
        reason: "numbers the kernel's table keeps for calls it never implemented or has \
                 removed, which it answers `ENOSYS` itself",
        verdict: Verdict::Refuse(Refusal::Absent),
        calls: &[
            call(156, "_sysctl"),
            call(174, "create_module"),
            call(177, "get_kernel_syms"),
            call(178, "query_module"),
            call(180, "nfsservctl"),
            call(181, "getpmsg"),
            call(182, "putpmsg"),
            call(183, "afs_syscall"),
            call(184, "tuxcall"),
            call(185, "security"),
            call(214, "epoll_ctl_old"),
            call(215, "epoll_wait_old"),
            call(236, "vserver"),
        ],
    },
    // the terminal, and availability
    Family {
        name: "ioctl",
        reason: "every request but injecting terminal input, the only layer for it where \
                 the `dev.tty.legacy_tiocsti` sysctl allows it: a stated gap for the requests \
                 that set a file's attribute flags (`FS_IOC_SETFLAGS`, \
                 `FS_IOC_FSSETXATTR`) on a file the user owns and the box can open, as \
                 for mode and time",
        verdict: Verdict::Ruled(ArgRule::Ioctl),
        calls: &[call(16, "ioctl")],
    },
    Family {
        name: "personality",
        reason: "reading the execution domain, never setting a flag such as \
                 `READ_IMPLIES_EXEC` or `ADDR_NO_RANDOMIZE` for what the process executes",
        verdict: Verdict::Ruled(ArgRule::Personality),
        calls: &[call(135, "personality")],
    },
    Family {
        name: "prlimit64",
        reason: "availability, which the threat model leaves outside: a process's own limits, \
                 never another's, since the kernel lets a process lower the limits of any of \
                 the same user's, outside the box too",
        verdict: Verdict::Ruled(ArgRule::Prlimit),
        calls: &[call(302, "prlimit64")],
    },
    // allowed
    Family {
        name: "files and directories",
        reason: "opening, making, removing, linking, and renaming by path, which Landlock \
                 gates by the box's rules; reading a path's metadata, whether it exists, \
                 and a link's target, and changing directory, which it doesn't gate, for \
                 any path the user can reach",
        verdict: Verdict::Allow,
        calls: &[
            call(2, "open"),
            call(4, "stat"),
            call(5, "fstat"),
            call(6, "lstat"),
            call(21, "access"),
            call(73, "flock"),
            call(76, "truncate"),
            call(78, "getdents"),
            call(79, "getcwd"),
            call(80, "chdir"),
            call(81, "fchdir"),
            call(82, "rename"),
            call(83, "mkdir"),
            call(84, "rmdir"),
            call(85, "creat"),
            call(86, "link"),
            call(87, "unlink"),
            call(88, "symlink"),
            call(89, "readlink"),
            call(95, "umask"),
            call(133, "mknod"),
            call(137, "statfs"),
            call(138, "fstatfs"),
            call(217, "getdents64"),
            call(257, "openat"),
            call(258, "mkdirat"),
            call(259, "mknodat"),
            call(262, "newfstatat"),
            call(263, "unlinkat"),
            call(264, "renameat"),
            call(265, "linkat"),
            call(266, "symlinkat"),
            call(267, "readlinkat"),
            call(269, "faccessat"),
            call(316, "renameat2"),
            call(332, "statx"),
            call(437, "openat2"),
            call(439, "faccessat2"),
        ],
    },
    Family {
        name: "mode and time",
        reason: "a file's mode, times, and attribute flags, which Landlock doesn't gate: a \
                 stated gap, since tools and tests set modes and times on the files they \
                 write (git's own config among them), so a boxed process can re-mode or \
                 retime any file the user owns that it can name",
        verdict: Verdict::Allow,
        calls: &[
            call(90, "chmod"),
            call(91, "fchmod"),
            call(132, "utime"),
            call(235, "utimes"),
            call(261, "futimesat"),
            call(268, "fchmodat"),
            call(280, "utimensat"),
            call(452, "fchmodat2"),
            call(469, "file_setattr"),
        ],
    },
    Family {
        name: "attribute reads",
        reason: "reading a file's extended attributes and attribute flags, which Landlock \
                 doesn't gate by path: for any path the user can reach",
        verdict: Verdict::Allow,
        calls: &[
            call(191, "getxattr"),
            call(192, "lgetxattr"),
            call(193, "fgetxattr"),
            call(194, "listxattr"),
            call(195, "llistxattr"),
            call(196, "flistxattr"),
            call(464, "getxattrat"),
            call(465, "listxattrat"),
            call(468, "file_getattr"),
        ],
    },
    Family {
        name: "descriptors and I/O",
        reason: "reading, writing, waiting on, and duplicating descriptors the process holds, \
                 the event loop's calls, and inotify's watches, which Landlock doesn't gate: a \
                 watch reports the names of entries made, changed, and removed in any directory \
                 the user can reach, one the box can't list among them, a stated gap that `gro \
                 test -- --watch` needs",
        verdict: Verdict::Allow,
        calls: &[
            call(0, "read"),
            call(1, "write"),
            call(3, "close"),
            call(7, "poll"),
            call(8, "lseek"),
            call(17, "pread64"),
            call(18, "pwrite64"),
            call(19, "readv"),
            call(20, "writev"),
            call(22, "pipe"),
            call(23, "select"),
            call(32, "dup"),
            call(33, "dup2"),
            call(40, "sendfile"),
            call(72, "fcntl"),
            call(74, "fsync"),
            call(75, "fdatasync"),
            call(77, "ftruncate"),
            call(162, "sync"),
            call(187, "readahead"),
            call(213, "epoll_create"),
            call(221, "fadvise64"),
            call(232, "epoll_wait"),
            call(233, "epoll_ctl"),
            call(253, "inotify_init"),
            call(254, "inotify_add_watch"),
            call(255, "inotify_rm_watch"),
            call(270, "pselect6"),
            call(271, "ppoll"),
            call(275, "splice"),
            call(276, "tee"),
            call(277, "sync_file_range"),
            call(281, "epoll_pwait"),
            call(282, "signalfd"),
            call(283, "timerfd_create"),
            call(284, "eventfd"),
            call(285, "fallocate"),
            call(286, "timerfd_settime"),
            call(287, "timerfd_gettime"),
            call(289, "signalfd4"),
            call(290, "eventfd2"),
            call(291, "epoll_create1"),
            call(292, "dup3"),
            call(293, "pipe2"),
            call(294, "inotify_init1"),
            call(295, "preadv"),
            call(296, "pwritev"),
            call(306, "syncfs"),
            call(319, "memfd_create"),
            call(326, "copy_file_range"),
            call(327, "preadv2"),
            call(328, "pwritev2"),
            call(436, "close_range"),
            call(441, "epoll_pwait2"),
            call(451, "cachestat"),
        ],
    },
    Family {
        name: "sockets",
        reason: "using a socket once made, which only the `socket` and `socketpair` rules \
                 decide",
        verdict: Verdict::Allow,
        calls: &[
            call(42, "connect"),
            call(43, "accept"),
            call(44, "sendto"),
            call(45, "recvfrom"),
            call(46, "sendmsg"),
            call(47, "recvmsg"),
            call(48, "shutdown"),
            call(49, "bind"),
            call(50, "listen"),
            call(51, "getsockname"),
            call(52, "getpeername"),
            call(54, "setsockopt"),
            call(55, "getsockopt"),
            call(288, "accept4"),
            call(299, "recvmmsg"),
            call(307, "sendmmsg"),
        ],
    },
    Family {
        name: "memory",
        reason: "the process's own memory: the run's box polices no executable mapping, its \
                 engine's JIT being on",
        verdict: Verdict::Allow,
        calls: &[
            call(9, "mmap"),
            call(10, "mprotect"),
            call(11, "munmap"),
            call(12, "brk"),
            call(25, "mremap"),
            call(26, "msync"),
            call(27, "mincore"),
            call(28, "madvise"),
            call(149, "mlock"),
            call(150, "munlock"),
            call(151, "mlockall"),
            call(152, "munlockall"),
            call(324, "membarrier"),
            call(325, "mlock2"),
            call(329, "pkey_mprotect"),
            call(330, "pkey_alloc"),
            call(331, "pkey_free"),
            call(453, "map_shadow_stack"),
            call(462, "mseal"),
        ],
    },
    Family {
        name: "processes, threads, and signals",
        reason: "running programs and waiting on them, signals (Landlock's signal scope \
                 keeps them inside the box), futexes, and a process's own state; \
                 `seccomp` and Landlock's calls only narrow a process further",
        verdict: Verdict::Allow,
        calls: &[
            call(13, "rt_sigaction"),
            call(14, "rt_sigprocmask"),
            call(15, "rt_sigreturn"),
            call(34, "pause"),
            call(36, "getitimer"),
            call(37, "alarm"),
            call(38, "setitimer"),
            call(39, "getpid"),
            call(57, "fork"),
            call(58, "vfork"),
            call(59, "execve"),
            call(60, "exit"),
            call(61, "wait4"),
            call(62, "kill"),
            call(109, "setpgid"),
            call(110, "getppid"),
            call(111, "getpgrp"),
            call(112, "setsid"),
            call(121, "getpgid"),
            call(124, "getsid"),
            call(127, "rt_sigpending"),
            call(128, "rt_sigtimedwait"),
            call(129, "rt_sigqueueinfo"),
            call(130, "rt_sigsuspend"),
            call(131, "sigaltstack"),
            call(157, "prctl"),
            call(158, "arch_prctl"),
            call(186, "gettid"),
            call(200, "tkill"),
            call(202, "futex"),
            call(205, "set_thread_area"),
            call(211, "get_thread_area"),
            call(218, "set_tid_address"),
            call(219, "restart_syscall"),
            call(231, "exit_group"),
            call(234, "tgkill"),
            call(247, "waitid"),
            call(273, "set_robust_list"),
            call(274, "get_robust_list"),
            call(297, "rt_tgsigqueueinfo"),
            call(317, "seccomp"),
            call(322, "execveat"),
            call(334, "rseq"),
            // the uprobe trampoline's return, which the kernel runs without
            // consulting the filter
            call(335, "uretprobe"),
            call(424, "pidfd_send_signal"),
            call(434, "pidfd_open"),
            call(444, "landlock_create_ruleset"),
            call(445, "landlock_add_rule"),
            call(446, "landlock_restrict_self"),
            call(449, "futex_waitv"),
            call(454, "futex_wake"),
            call(455, "futex_wait"),
            call(456, "futex_requeue"),
        ],
    },
    Family {
        name: "credentials",
        reason: "the process's own user, groups, and capabilities, which a process without \
                 a capability can read and only narrow",
        verdict: Verdict::Allow,
        calls: &[
            call(102, "getuid"),
            call(104, "getgid"),
            call(105, "setuid"),
            call(106, "setgid"),
            call(107, "geteuid"),
            call(108, "getegid"),
            call(113, "setreuid"),
            call(114, "setregid"),
            call(115, "getgroups"),
            call(116, "setgroups"),
            call(117, "setresuid"),
            call(118, "getresuid"),
            call(119, "setresgid"),
            call(120, "getresgid"),
            call(122, "setfsuid"),
            call(123, "setfsgid"),
            call(125, "capget"),
            call(126, "capset"),
        ],
    },
    Family {
        name: "time",
        reason: "reading the clock, sleeping, and a process's own timers",
        verdict: Verdict::Allow,
        calls: &[
            call(35, "nanosleep"),
            call(96, "gettimeofday"),
            call(100, "times"),
            call(201, "time"),
            call(222, "timer_create"),
            call(223, "timer_settime"),
            call(224, "timer_gettime"),
            call(225, "timer_getoverrun"),
            call(226, "timer_delete"),
            call(228, "clock_gettime"),
            call(229, "clock_getres"),
            call(230, "clock_nanosleep"),
        ],
    },
    Family {
        name: "scheduling and resource use",
        reason: "a process's priority, scheduling, affinity, and limits: a stated gap for \
                 another of the user's processes, whose priority and scheduling the kernel \
                 lets the same user lower, which reaches its availability and nothing else \
                 (the calls name a thread by its id, so a filter can't tell the caller's \
                 own threads from another process)",
        verdict: Verdict::Allow,
        calls: &[
            call(24, "sched_yield"),
            call(97, "getrlimit"),
            call(98, "getrusage"),
            call(140, "getpriority"),
            call(141, "setpriority"),
            call(142, "sched_setparam"),
            call(143, "sched_getparam"),
            call(144, "sched_setscheduler"),
            call(145, "sched_getscheduler"),
            call(146, "sched_get_priority_max"),
            call(147, "sched_get_priority_min"),
            call(148, "sched_rr_get_interval"),
            call(160, "setrlimit"),
            call(203, "sched_setaffinity"),
            call(204, "sched_getaffinity"),
            call(251, "ioprio_set"),
            call(252, "ioprio_get"),
            call(309, "getcpu"),
            call(314, "sched_setattr"),
            call(315, "sched_getattr"),
        ],
    },
    Family {
        name: "system information",
        reason: "reading what the system is: its name, load, and random bytes; and the \
                 process's own security module attributes, read and set as the module's policy \
                 allows",
        verdict: Verdict::Allow,
        calls: &[
            call(63, "uname"),
            call(99, "sysinfo"),
            call(136, "ustat"),
            call(318, "getrandom"),
            call(459, "lsm_get_self_attr"),
            call(460, "lsm_set_self_attr"),
            call(461, "lsm_list_modules"),
        ],
    },
];

/// The denylist's filter for a box whose network is held in the form `net`.
///
/// It's the table assembled by the tree assembler, every number above the
/// highest classified failing with `ENOSYS`, and a call from another
/// architecture killing the process.
///
/// # Errors
///
/// Returns the [`TreeError`] for a table the assembler can't make a filter
/// of: a number classified twice or left unclassified, or a program longer
/// than the kernel takes. The table's tests hold that it is neither.
pub fn denylist_filter(net: NetForm) -> Result<BpfProgram, TreeError> {
    let unassigned = UNASSIGNED.map(|nr| (nr, Verdict::Refuse(Refusal::Absent)));
    let classified = || {
        FAMILIES
            .iter()
            .flat_map(|family| family.calls.iter().map(|call| (call.nr, family.verdict)))
            .chain(unassigned.clone())
    };
    let highest = classified().map(|(nr, _)| nr).max().unwrap_or(0);
    // each verdict's decision made once, and each number's slot its index
    let mut decisions: Vec<(Verdict, Decision)> = Vec::new();
    let mut slots: Vec<Option<usize>> = vec![None; slot(highest) + 1];
    for (nr, verdict) in classified() {
        let made = decisions.iter().position(|(made, _)| *made == verdict);
        let index = made.unwrap_or_else(|| {
            decisions.push((verdict, decision(verdict, net)));
            decisions.len() - 1
        });
        if slots[slot(nr)].replace(index).is_some() {
            return Err(TreeError::Twice { nr });
        }
    }
    let calls = (0..=highest)
        .zip(slots)
        .map(|(nr, slot)| {
            slot.map(|index| &decisions[index].1)
                .ok_or(TreeError::Unclassified { nr })
        })
        .collect::<Result<Vec<&Decision>, TreeError>>()?;
    tree::assemble(AUDIT_ARCH_X86_64, &calls, errno(Refusal::Absent))
}

/// A call number's index in a table that starts at 0.
fn slot(nr: u32) -> usize {
    usize::try_from(nr).unwrap_or(usize::MAX)
}

const fn errno(refusal: Refusal) -> Action {
    Action::Errno(refusal.errno().unsigned_abs())
}

/// An int's bits, as the kernel reads the low half of an argument.
const fn int(value: libc::c_int) -> u32 {
    value.cast_unsigned()
}

/// How `verdict` decides a call in a box whose network is held as `net`.
fn decision(verdict: Verdict, net: NetForm) -> Decision {
    let allow = || Decision::Return(Action::Allow);
    let refuse = || Decision::Return(errno(Refusal::NotPermitted));
    let check = |arg, mask, cases, otherwise| {
        Decision::Match(Box::new(ArgMatch {
            arg,
            mask,
            cases,
            otherwise,
        }))
    };
    match verdict {
        Verdict::Allow => allow(),
        Verdict::Refuse(refusal) => Decision::Return(errno(refusal)),
        // the flags, the first argument: none of the namespace flags
        Verdict::Ruled(ArgRule::Clone) => {
            check(0, int(CLONE_NAMESPACES), vec![(0, allow())], refuse())
        }
        // the request, the second argument
        Verdict::Ruled(ArgRule::Ioctl) => check(
            1,
            u32::MAX,
            REFUSED_IOCTLS
                .iter()
                .map(|request| (ioctl_low(*request), refuse()))
                .collect(),
            allow(),
        ),
        // the persona, the first argument
        Verdict::Ruled(ArgRule::Personality) => check(
            0,
            u32::MAX,
            KEPT_PERSONALITIES
                .iter()
                .map(|persona| (*persona, allow()))
                .collect(),
            refuse(),
        ),
        // the pid, the first argument
        Verdict::Ruled(ArgRule::Prlimit) => check(0, u32::MAX, vec![(0, allow())], refuse()),
        // the domain, the first argument, then the type, the second, its
        // flags masked off, then the protocol, the third
        Verdict::Ruled(ArgRule::Socket) => match net {
            NetForm::Namespace => {
                let mask = u32::try_from(SOCKET_TYPE_MASK).unwrap_or(u32::MAX);
                let kinds = || {
                    KEPT_INET_SOCKETS
                        .iter()
                        .map(|(kind, protocol)| {
                            let protocols = vec![(0, allow()), (int(*protocol), allow())];
                            (int(*kind), check(2, u32::MAX, protocols, refuse()))
                        })
                        .collect()
                };
                check(
                    0,
                    u32::MAX,
                    INET_DOMAINS
                        .iter()
                        .map(|domain| (int(*domain), check(1, mask, kinds(), refuse())))
                        .collect(),
                    refuse(),
                )
            }
            NetForm::InetRefused => refuse(),
        },
        // the domain, the first argument, then the type, the second, its
        // flags masked off
        Verdict::Ruled(ArgRule::Socketpair) => {
            let mut domains: Vec<(u32, Vec<(u32, Decision)>)> = Vec::new();
            for (domain, kind) in KEPT_SOCKETPAIRS {
                let kind = (int(kind), allow());
                match domains.iter_mut().find(|(kept, _)| *kept == int(domain)) {
                    Some((_, kinds)) => kinds.push(kind),
                    None => domains.push((int(domain), vec![kind])),
                }
            }
            let mask = u32::try_from(SOCKET_TYPE_MASK).unwrap_or(u32::MAX);
            check(
                0,
                u32::MAX,
                domains
                    .into_iter()
                    .map(|(domain, kinds)| (domain, check(1, mask, kinds, refuse())))
                    .collect(),
                refuse(),
            )
        }
    }
}

/// An `ioctl` request's low 32 bits, the `unsigned int` the kernel reads.
fn ioctl_low(request: libc::Ioctl) -> u32 {
    u32::try_from(ioctl_request(request) & u64::from(u32::MAX)).unwrap_or(u32::MAX)
}
