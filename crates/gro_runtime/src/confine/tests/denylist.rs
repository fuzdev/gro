//! Tests for the run's seccomp denylist: the table's census against the
//! kernel's headers, and the filter for each network form read by the BPF
//! interpreter for every call number and representative arguments.

use std::collections::{BTreeMap, BTreeSet};

use super::bpf::{check_classic, run};
use crate::confine::NetForm;
use crate::confine::seccomp::denylist::{
    ArgRule, FAMILIES, INET_DOMAINS, KEPT_INET_SOCKETS, KEPT_PERSONALITIES, KEPT_SOCKETPAIRS,
    REFUSED_IOCTLS, Refusal, UNASSIGNED, Verdict, denylist_filter,
};

/// The highest call number the table classifies: `file_setattr`.
const HIGHEST: u32 = 469;

const ARCH: u32 = 0xc000_003e;
const ALLOW: u32 = libc::SECCOMP_RET_ALLOW;
const KILL: u32 = libc::SECCOMP_RET_KILL_PROCESS;
const ENOSYS: u32 = libc::SECCOMP_RET_ERRNO | libc::ENOSYS.unsigned_abs();
const EPERM: u32 = libc::SECCOMP_RET_ERRNO | libc::EPERM.unsigned_abs();

/// The headers naming the kernel's `x86_64` call numbers, Debian's and
/// Ubuntu's path first, then the plain one.
const HEADERS: [&str; 2] = [
    "/usr/include/x86_64-linux-gnu/asm/unistd_64.h",
    "/usr/include/asm/unistd_64.h",
];

/// A call number from libc's name for it.
fn nr(call: libc::c_long) -> u32 {
    u32::try_from(call).unwrap()
}

/// The calls refused `ENOSYS` whole, listed apart from the table, so a call
/// or a family dropped there, or its refusal swapped, fails here: by family,
/// with the numbers libc doesn't name written out.
fn absent_calls() -> Vec<u32> {
    let named = [
        // other processes
        libc::SYS_ptrace,
        libc::SYS_process_vm_readv,
        libc::SYS_process_vm_writev,
        libc::SYS_kcmp,
        libc::SYS_pidfd_getfd,
        libc::SYS_process_madvise,
        libc::SYS_process_mrelease,
        // namespaces and mounts
        libc::SYS_pivot_root,
        libc::SYS_chroot,
        libc::SYS_mount,
        libc::SYS_umount2,
        libc::SYS_unshare,
        libc::SYS_setns,
        libc::SYS_open_tree,
        libc::SYS_move_mount,
        libc::SYS_fsopen,
        libc::SYS_fsconfig,
        libc::SYS_fsmount,
        libc::SYS_fspick,
        libc::SYS_clone3,
        libc::SYS_mount_setattr,
        // the kernel's surface
        libc::SYS_modify_ldt,
        libc::SYS_init_module,
        libc::SYS_finit_module,
        libc::SYS_delete_module,
        libc::SYS_kexec_load,
        libc::SYS_kexec_file_load,
        libc::SYS_add_key,
        libc::SYS_request_key,
        libc::SYS_keyctl,
        libc::SYS_perf_event_open,
        libc::SYS_name_to_handle_at,
        libc::SYS_open_by_handle_at,
        libc::SYS_bpf,
        libc::SYS_userfaultfd,
        libc::SYS_io_uring_setup,
        libc::SYS_io_uring_enter,
        libc::SYS_io_uring_register,
        // system administration
        libc::SYS_syslog,
        libc::SYS_uselib,
        libc::SYS_vhangup,
        libc::SYS_adjtimex,
        libc::SYS_acct,
        libc::SYS_settimeofday,
        libc::SYS_swapon,
        libc::SYS_swapoff,
        libc::SYS_reboot,
        libc::SYS_sethostname,
        libc::SYS_setdomainname,
        libc::SYS_iopl,
        libc::SYS_ioperm,
        libc::SYS_quotactl,
        libc::SYS_quotactl_fd,
        libc::SYS_lookup_dcookie,
        libc::SYS_clock_settime,
        libc::SYS_clock_adjtime,
        // System V IPC and POSIX message queues
        libc::SYS_shmget,
        libc::SYS_shmat,
        libc::SYS_shmctl,
        libc::SYS_shmdt,
        libc::SYS_semget,
        libc::SYS_semop,
        libc::SYS_semctl,
        libc::SYS_semtimedop,
        libc::SYS_msgget,
        libc::SYS_msgsnd,
        libc::SYS_msgrcv,
        libc::SYS_msgctl,
        libc::SYS_mq_open,
        libc::SYS_mq_unlink,
        libc::SYS_mq_timedsend,
        libc::SYS_mq_timedreceive,
        libc::SYS_mq_notify,
        libc::SYS_mq_getsetattr,
        // never seen called: asynchronous I/O, NUMA placement, and the rest
        libc::SYS_io_setup,
        libc::SYS_io_destroy,
        libc::SYS_io_getevents,
        libc::SYS_io_submit,
        libc::SYS_io_cancel,
        libc::SYS_mbind,
        libc::SYS_set_mempolicy,
        libc::SYS_get_mempolicy,
        libc::SYS_migrate_pages,
        libc::SYS_move_pages,
        libc::SYS_set_mempolicy_home_node,
        libc::SYS_remap_file_pages,
        libc::SYS_memfd_secret,
        libc::SYS_vmsplice,
        libc::SYS_fanotify_init,
        libc::SYS_fanotify_mark,
        libc::SYS_sysfs,
        // never implemented or removed
        libc::SYS__sysctl,
        libc::SYS_nfsservctl,
        libc::SYS_getpmsg,
        libc::SYS_putpmsg,
        libc::SYS_afs_syscall,
        libc::SYS_tuxcall,
        libc::SYS_security,
        libc::SYS_epoll_ctl_old,
        libc::SYS_epoll_wait_old,
        libc::SYS_vserver,
    ];
    let mut calls: Vec<u32> = named.into_iter().map(nr).collect();
    // `statmount`, `listmount`, `open_tree_attr`, `io_pgetevents`, and
    // `create_module`, `get_kernel_syms`, `query_module`
    calls.extend([457, 458, 467, 333, 174, 177, 178]);
    calls.extend(UNASSIGNED_NUMBERS);
    calls
}

/// The numbers no `x86_64` call has, written apart from [`UNASSIGNED`].
const UNASSIGNED_NUMBERS: std::ops::RangeInclusive<u32> = 336..=423;

/// The calls refused `EPERM` whole: the ownership and extended-attribute
/// calls.
fn not_permitted_calls() -> Vec<u32> {
    let mut calls: Vec<u32> = [
        libc::SYS_chown,
        libc::SYS_fchown,
        libc::SYS_lchown,
        libc::SYS_fchownat,
        libc::SYS_setxattr,
        libc::SYS_lsetxattr,
        libc::SYS_fsetxattr,
        libc::SYS_removexattr,
        libc::SYS_lremovexattr,
        libc::SYS_fremovexattr,
    ]
    .into_iter()
    .map(nr)
    .collect();
    // `setxattrat` and `removexattrat`
    calls.extend([463, 466]);
    calls
}

/// The calls ruled by their arguments, each with its rule.
fn ruled_calls() -> [(u32, ArgRule); 6] {
    [
        (nr(libc::SYS_clone), ArgRule::Clone),
        (nr(libc::SYS_ioctl), ArgRule::Ioctl),
        (nr(libc::SYS_personality), ArgRule::Personality),
        (nr(libc::SYS_prlimit64), ArgRule::Prlimit),
        (nr(libc::SYS_socket), ArgRule::Socket),
        (nr(libc::SYS_socketpair), ArgRule::Socketpair),
    ]
}

/// The table's verdict for each number it classifies, the unassigned ones
/// included.
fn table() -> BTreeMap<u32, Verdict> {
    let mut table = BTreeMap::new();
    for family in FAMILIES {
        for call in family.calls {
            assert!(
                table.insert(call.nr, family.verdict).is_none(),
                "{} ({}) classified twice",
                call.name,
                call.nr
            );
        }
    }
    for number in UNASSIGNED {
        assert!(
            table
                .insert(number, Verdict::Refuse(Refusal::Absent))
                .is_none(),
            "{number} classified twice"
        );
    }
    table
}

/// Every number from 0 to the highest is classified once, each family's
/// calls are in ascending order and named once, and every family has a
/// reason.
#[test]
fn the_table_classifies_every_number_once() {
    let table = table();
    assert_eq!(
        table.keys().copied().collect::<Vec<_>>(),
        (0..=HIGHEST).collect::<Vec<_>>()
    );
    assert_eq!(UNASSIGNED, UNASSIGNED_NUMBERS);
    let mut names = BTreeSet::new();
    for family in FAMILIES {
        assert!(!family.reason.is_empty(), "{}", family.name);
        assert!(
            family.calls.is_sorted_by_key(|call| call.nr),
            "{}'s calls are out of order",
            family.name
        );
        for call in family.calls {
            assert!(names.insert(call.name), "{} named twice", call.name);
        }
    }
}

/// The table names every call the kernel's headers on this machine name,
/// by the same number, and classifies no number above the highest; a
/// header naming a call above it fails here until the table classifies it.
#[test]
fn the_table_names_the_kernel_headers_calls() {
    let (path, header) = HEADERS
        .iter()
        .find_map(|path| Some((path, std::fs::read_to_string(path).ok()?)))
        .unwrap_or_else(|| {
            panic!(
                "no kernel header names the x86_64 call numbers (tried {HEADERS:?}): install the \
                 kernel's userspace headers (Debian's and Ubuntu's `linux-libc-dev`)"
            )
        });
    let named: BTreeMap<&str, u32> = FAMILIES
        .iter()
        .flat_map(|family| family.calls)
        .map(|call| (call.name, call.nr))
        .collect();
    let mut count = 0;
    for line in header.lines() {
        let Some(rest) = line.strip_prefix("#define __NR_") else {
            continue;
        };
        let (name, number) = rest.split_once(' ').unwrap();
        let number: u32 = number.trim().parse().unwrap();
        assert!(
            number <= HIGHEST,
            "{path} names `{name}` ({number}), above the denylist's highest classified call \
             ({HIGHEST}): classify it in the table, with its family's reason"
        );
        assert_eq!(named.get(name), Some(&number), "{path}'s `{name}`");
        count += 1;
    }
    assert!(count > 300, "{path} named only {count} calls");
}

/// The rules' values, pinned.
#[test]
fn the_rules_values_are_pinned() {
    assert_eq!(REFUSED_IOCTLS, [libc::TIOCSTI, libc::TIOCLINUX]);
    assert_eq!(KEPT_PERSONALITIES, [0xffff_ffff, 0]);
    assert_eq!(INET_DOMAINS, [libc::AF_INET, libc::AF_INET6]);
    assert_eq!(
        KEPT_INET_SOCKETS,
        [
            (libc::SOCK_STREAM, libc::IPPROTO_TCP),
            (libc::SOCK_DGRAM, libc::IPPROTO_UDP)
        ]
    );
    assert_eq!(
        KEPT_SOCKETPAIRS,
        [
            (libc::AF_UNIX, libc::SOCK_STREAM),
            (libc::AF_UNIX, libc::SOCK_SEQPACKET)
        ]
    );
    let ruled: Vec<(u32, ArgRule)> = table()
        .into_iter()
        .filter_map(|(number, verdict)| match verdict {
            Verdict::Ruled(rule) => Some((number, rule)),
            Verdict::Allow | Verdict::Refuse(_) => None,
        })
        .collect();
    let mut expected = ruled_calls().to_vec();
    expected.sort_unstable_by_key(|(number, _)| *number);
    assert_eq!(ruled, expected);
}

/// What the filter answers each ruled call with each set of arguments, by
/// network form: every argument compared by its low half, so an upper half
/// the kernel drops changes nothing, in either direction.
fn ruled_cases(net: NetForm) -> Vec<(u32, [u64; 6], u32)> {
    let high = 0xdead_0000_0000_0000_u64;
    let int = |value: libc::c_int| u64::from(value.cast_unsigned());
    let args = |first: u64, second: u64| [first, second, 0, 0, 0, 0];
    let mut cases = Vec::new();
    // `clone`: glibc's flags for a thread, a fork, and a spawn pass, as
    // does `CLONE_NEWTIME`'s bit, which `clone` reads as the exit signal's;
    // a namespace flag is refused, beside any other flag
    let clone = nr(libc::SYS_clone);
    let thread = int(libc::CLONE_VM
        | libc::CLONE_FS
        | libc::CLONE_FILES
        | libc::CLONE_SIGHAND
        | libc::CLONE_THREAD
        | libc::CLONE_SYSVSEM
        | libc::CLONE_SETTLS
        | libc::CLONE_PARENT_SETTID
        | libc::CLONE_CHILD_CLEARTID);
    let sigchld = int(libc::SIGCHLD);
    let fork = int(libc::CLONE_CHILD_SETTID | libc::CLONE_CHILD_CLEARTID) | sigchld;
    let spawn = int(libc::CLONE_VM | libc::CLONE_VFORK) | sigchld;
    for flags in [
        thread,
        fork,
        spawn,
        0,
        high | thread,
        0x80,
        u64::from(u32::MAX) << 32,
    ] {
        cases.push((clone, args(flags, 0), ALLOW));
    }
    for namespace in [
        libc::CLONE_NEWNS,
        libc::CLONE_NEWCGROUP,
        libc::CLONE_NEWUTS,
        libc::CLONE_NEWIPC,
        libc::CLONE_NEWUSER,
        libc::CLONE_NEWPID,
        libc::CLONE_NEWNET,
    ] {
        let namespace = int(namespace);
        for flags in [
            namespace,
            thread | namespace,
            fork | namespace,
            high | namespace,
        ] {
            cases.push((clone, args(flags, 0), EPERM));
        }
    }
    // `ioctl`: the terminal's input injection refused, also with an upper
    // half the kernel drops; every other request passes, the same request
    // in the upper half among them
    let ioctl = nr(libc::SYS_ioctl);
    for request in [libc::TIOCSTI, libc::TIOCLINUX] {
        cases.push((ioctl, args(0, request), EPERM));
        cases.push((ioctl, args(0, high | request), EPERM));
        cases.push((ioctl, args(0, request << 32), ALLOW));
        cases.push((ioctl, args(request, 0), ALLOW));
    }
    for request in [
        libc::FIONREAD,
        libc::FIONBIO,
        libc::TCGETS,
        libc::TCSETS,
        libc::TIOCGWINSZ,
        libc::TIOCSWINSZ,
        libc::FS_IOC_SETFLAGS,
        0,
    ] {
        cases.push((ioctl, args(0, request), ALLOW));
    }
    // `personality`: a query and `PER_LINUX` pass, also with an upper half;
    // a flag or another domain is refused
    let personality = nr(libc::SYS_personality);
    for persona in [0xffff_ffff, 0, high | 0xffff_ffff, high, 1 << 32] {
        cases.push((personality, args(persona, 0), ALLOW));
    }
    // `READ_IMPLIES_EXEC`, `ADDR_NO_RANDOMIZE`, `PER_LINUX32`,
    // `ADDR_LIMIT_32BIT`, and the query's value less one
    for persona in [0x040_0000, 0x004_0000, 0x0008, 0x080_0000, 0xffff_fffe] {
        cases.push((personality, args(persona, 0), EPERM));
        cases.push((personality, args(high | persona, 0), EPERM));
    }
    // `prlimit64`: the caller's own limits, pid 0, also with an upper half;
    // any other pid refused, -1 and one whose low half is 0's aside
    let prlimit = nr(libc::SYS_prlimit64);
    for pid in [0, high, 1 << 32] {
        cases.push((prlimit, args(pid, 7), ALLOW));
    }
    for pid in [1, 2, 4_194_304, u64::from(u32::MAX), high | 1, u64::MAX] {
        cases.push((prlimit, args(pid, 7), EPERM));
    }
    // `socket`: in the namespace, a TCP stream or a UDP datagram socket,
    // its protocol named or 0 and its type's flags masked off, also with an
    // upper half in any argument; none where the run has none. A raw
    // socket, another protocol (SCTP's, DCCP's, `UDPLITE`'s, ICMP's ping),
    // and a type with another's protocol refused, as the kernel never sees
    // them
    let socket = nr(libc::SYS_socket);
    let inet = match net {
        NetForm::Namespace => ALLOW,
        NetForm::InetRefused => EPERM,
    };
    let three = |domain: u64, kind: u64, protocol: u64| [domain, kind, protocol, 0, 0, 0];
    let [stream, datagram] = [libc::SOCK_STREAM, libc::SOCK_DGRAM].map(int);
    let [tcp, udp] = [libc::IPPROTO_TCP, libc::IPPROTO_UDP].map(int);
    let type_flags = int(libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK);
    for domain in [libc::AF_INET, libc::AF_INET6].map(int) {
        for (kind, protocol) in [(stream, tcp), (datagram, udp)] {
            for allowed in [
                three(domain, kind, 0),
                three(domain, kind, protocol),
                three(domain, kind | type_flags, protocol),
                three(high | domain, high | kind, high | protocol),
                three(domain, kind, int(libc::IPPROTO_SCTP) << 32),
            ] {
                cases.push((socket, allowed, inet));
            }
        }
        for refused in [
            three(domain, int(libc::SOCK_RAW), 0),
            three(domain, int(libc::SOCK_RAW), int(libc::IPPROTO_RAW)),
            three(domain, int(libc::SOCK_SEQPACKET), 0),
            three(domain, stream, int(libc::IPPROTO_SCTP)),
            three(domain, int(libc::SOCK_SEQPACKET), int(libc::IPPROTO_SCTP)),
            // `SOCK_DCCP` and `IPPROTO_DCCP`
            three(domain, 6, 33),
            three(domain, datagram, int(libc::IPPROTO_UDPLITE)),
            three(domain, datagram, int(libc::IPPROTO_ICMP)),
            three(domain, stream, udp),
            three(domain, datagram, tcp),
            three(domain, stream, high | int(libc::IPPROTO_SCTP)),
            three(domain, stream << 32, 0),
        ] {
            cases.push((socket, refused, EPERM));
        }
    }
    // `AF_UNIX`, `AF_NETLINK`, `AF_PACKET`, `AF_ALG`, `AF_VSOCK`, the
    // unspecified one, and `AF_INET` in the upper half
    for domain in [
        int(libc::AF_UNIX),
        int(libc::AF_NETLINK),
        int(libc::AF_PACKET),
        38,
        40,
        0,
        int(libc::AF_INET) << 32,
        high | int(libc::AF_UNIX),
    ] {
        cases.push((socket, args(domain, int(libc::SOCK_STREAM)), EPERM));
    }
    // `socketpair`: a unix stream or sequenced-packet pair, its flags
    // masked off and also with an upper half; a datagram pair and any
    // other domain refused
    let socketpair = nr(libc::SYS_socketpair);
    let unix = int(libc::AF_UNIX);
    let flags = int(libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK);
    for kind in [libc::SOCK_STREAM, libc::SOCK_SEQPACKET] {
        let kind = int(kind);
        for pair in [
            args(unix, kind),
            args(unix, kind | flags),
            args(high | unix, high | kind),
        ] {
            cases.push((socketpair, pair, ALLOW));
        }
        cases.push((socketpair, args(int(libc::AF_INET), kind), EPERM));
        cases.push((socketpair, args(unix << 32, kind), EPERM));
    }
    for kind in [libc::SOCK_DGRAM, libc::SOCK_RAW, 0] {
        cases.push((socketpair, args(unix, int(kind)), EPERM));
        cases.push((socketpair, args(unix, int(kind) | flags), EPERM));
    }
    cases
}

/// The filter for each network form decides every number as the lists
/// above say, read as the kernel reads it: the absent calls and the
/// unassigned numbers fail with `ENOSYS`, the ownership and
/// extended-attribute calls with `EPERM`, whatever their arguments; the
/// ruled calls by their cases; every other number up to the highest is
/// allowed, whatever its arguments; every number above the highest fails
/// with `ENOSYS`, the x32 calls among them; and a call from another
/// architecture kills.
#[test]
fn the_filter_decides_by_the_table() {
    let absent: BTreeSet<u32> = absent_calls().into_iter().collect();
    let not_permitted: BTreeSet<u32> = not_permitted_calls().into_iter().collect();
    let ruled: BTreeSet<u32> = ruled_calls().map(|(number, _)| number).into();
    assert!(absent.is_disjoint(&not_permitted) && absent.is_disjoint(&ruled));
    assert!(not_permitted.is_disjoint(&ruled));
    let any = [u64::MAX, 0xdead_0000_0000_0001, 1, 2, 3, 4];
    for net in [NetForm::Namespace, NetForm::InetRefused] {
        let filter = denylist_filter(net).unwrap();
        check_classic(&filter);
        let decide = |number: u32, args: [u64; 6]| run(&filter, ARCH, number, args).0;
        for number in 0..=HIGHEST {
            if ruled.contains(&number) {
                continue;
            }
            let expected = if absent.contains(&number) {
                ENOSYS
            } else if not_permitted.contains(&number) {
                EPERM
            } else {
                ALLOW
            };
            assert_eq!(decide(number, [0; 6]), expected, "{net:?} {number}");
            assert_eq!(decide(number, any), expected, "{net:?} {number} {any:x?}");
        }
        for (number, args, expected) in ruled_cases(net) {
            assert_eq!(decide(number, args), expected, "{net:?} {number} {args:x?}");
        }
        // above the highest, an x32 call, and numbers no table reaches
        for number in [HIGHEST + 1, HIGHEST + 2, 512, 547, 0x3fff_ffff, u32::MAX] {
            assert_eq!(decide(number, [0; 6]), ENOSYS, "{net:?} {number}");
        }
        for number in 0..=HIGHEST {
            let x32 = number | 0x4000_0000;
            assert_eq!(decide(x32, [0; 6]), ENOSYS, "{net:?} {x32:#x}");
            assert_eq!(decide(x32, any), ENOSYS, "{net:?} {x32:#x}");
        }
        // `AUDIT_ARCH_I386` and one bit off `x86_64`'s
        for arch in [0x4000_0003, ARCH ^ 1] {
            for number in [0, nr(libc::SYS_socket), nr(libc::SYS_ptrace), HIGHEST + 1] {
                assert_eq!(run(&filter, arch, number, [0; 6]).0, KILL, "{arch:#x}");
            }
        }
    }
}

/// The filter is a search, not a list: no call runs more than the
/// architecture check, the number's load, a comparison per level, and its
/// leaf's checks, whatever its number or arguments.
#[test]
fn the_filter_searches_by_number() {
    for net in [NetForm::Namespace, NetForm::InetRefused] {
        let filter = denylist_filter(net).unwrap();
        let mut deepest = 0;
        for number in (0..=HIGHEST + 2).chain([0x4000_0000, u32::MAX]) {
            deepest = deepest.max(run(&filter, ARCH, number, [0; 6]).1);
        }
        for (number, args, _) in ruled_cases(net) {
            deepest = deepest.max(run(&filter, ARCH, number, args).1);
        }
        // the arch check and the load, about eight levels, and `socket`'s
        // three checks, the deepest leaf
        assert!(deepest <= 24, "{net:?}: a call ran {deepest} instructions");
        assert!(
            filter.len() <= 512,
            "{net:?}: {} instructions",
            filter.len()
        );
    }
}
