//! `determinism: deny`: the refused operations, the absent timers, and the time
//! zone check.

use std::process::Command;

use serde_json::json;

use crate::support::Scratch;
use crate::{driven, eval, eval_with, parse, spawn, wrapped_command};

/// The reason each operation `determinism: deny` refuses gives, and the
/// text after it, one per reason.
const WHY: [(&str, &str); 4] = [
    (
        "clock",
        "it reads the clock, so the value would differ from run to run; write the date or time \
         as a literal with its offset, such as new Date('2026-01-01T00:00:00Z')",
    ),
    (
        "random",
        "it is random, so the value would differ from run to run; write the value as a literal, \
         or derive it from the module's own data",
    ),
    (
        "machine",
        "it is a fact of this process or machine, so the value would differ from host to host",
    ),
    (
        "timer",
        "it schedules work on the clock, and eval has no timers; chain promise jobs instead",
    ),
];

/// The refusal of `shown` for `why`.
fn refusal(shown: &str, why: &str) -> String {
    let (_, text) = WHY.iter().find(|(reason, _)| *reason == why).unwrap();
    format!("{shown} is refused in eval ({why}): {text}")
}

/// Each operation `determinism: deny` refuses throws its refusal, naming
/// it and the reason, and no way back to its original runs: not its
/// descriptor, its prototype or constructor, `Reflect`, a subclass,
/// deleting the replacement, or (for the formatters) passing `undefined`
/// for the date. `Blob`, `File`, and the object URLs are absent. What's
/// kept still works, with each replaced member's attributes, and the
/// stand-ins are inert.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn determinism_deny_refuses_each_entry_and_keeps_no_original() {
    let run = eval(&["deny.js"]);
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    let value = &run.envelope["value"];
    let mut entries = vec![
        ("Math.random", "Math.random()", "random"),
        ("performance.now", "performance.now()", "clock"),
        ("performance.mark", "performance.mark()", "clock"),
        ("performance.measure", "performance.measure()", "clock"),
        ("performance.toJSON", "performance.toJSON()", "clock"),
        ("performance.timeOrigin", "performance.timeOrigin", "clock"),
        ("Date()", "Date()", "clock"),
        ("new Date()", "new Date()", "clock"),
        ("Date.now", "Date.now()", "clock"),
        (
            "Intl.DateTimeFormat format",
            "Intl.DateTimeFormat format() with no date",
            "clock",
        ),
        (
            "Intl.DateTimeFormat formatToParts",
            "Intl.DateTimeFormat formatToParts() with no date",
            "clock",
        ),
        ("AbortSignal.timeout", "AbortSignal.timeout()", "timer"),
        (
            "Atomics.waitAsync",
            "Atomics.waitAsync() with a timeout",
            "timer",
        ),
    ];
    let temporal = [
        ("Temporal.Now.instant", "Temporal.Now.instant()", "clock"),
        (
            "Temporal.Now.timeZoneId",
            "Temporal.Now.timeZoneId()",
            "machine",
        ),
        (
            "Temporal.Now.plainDateTimeISO",
            "Temporal.Now.plainDateTimeISO()",
            "clock",
        ),
        (
            "Temporal.Now.zonedDateTimeISO",
            "Temporal.Now.zonedDateTimeISO()",
            "clock",
        ),
        (
            "Temporal.Now.plainDateISO",
            "Temporal.Now.plainDateISO()",
            "clock",
        ),
        (
            "Temporal.Now.plainTimeISO",
            "Temporal.Now.plainTimeISO()",
            "clock",
        ),
    ];
    entries.extend(temporal);
    for (entry, shown, why) in &entries {
        assert_eq!(
            value[entry],
            json!({"refused": refusal(shown, why), "recovered": []}),
            "{entry}"
        );
    }
    // the fixture tried exactly these, so a new entry needs a line here
    let mut tried: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .filter(|key| !["deleted", "absent", "kept", "inert"].contains(key))
        .collect();
    tried.sort_unstable();
    let mut expected: Vec<&str> = entries.iter().map(|(entry, ..)| *entry).collect();
    expected.sort_unstable();
    assert_eq!(tried, expected);
    assert_eq!(value["deleted"], json!([]));
    assert_eq!(
        value["absent"],
        json!([
            "undefined",
            "undefined",
            "undefined",
            "undefined",
            false,
            false
        ])
    );
    assert_eq!(
        value["kept"],
        json!({
            "date": "1970-01-01T00:00:00.000Z",
            "utc": 1_767_225_600_000_u64,
            "parse": 1_767_225_600_000_u64,
            "format": "1/1/1970",
            "instanceof": true,
            "date_name": "Date/7",
            "wait_async_now": "not-equal",
            "wait_async_zero": "timed-out",
            "attributes": ["true/false/true", "true/false/true", "undefined/false/true"],
        })
    );
    assert_eq!(
        value["inert"],
        json!({
            "deref": true,
            "weak_ref_tag": "[object WeakRef]",
            "registry_tag": "[object FinalizationRegistry]",
            "unregister": true,
            "weak_ref_refuses": "invalid value used as a weak reference target",
            "console_time": "time/timeLog/timeEnd",
        })
    );
    // the console's timers printed nothing
    assert!(run.stderr.is_empty(), "{}", run.stderr);
}

/// The driver, evaluated before any of the module's code, already sees the
/// refusals and no timers.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn the_driver_sees_determinism_deny() {
    driven("value.js", "deny.js").assert_value(&json!({
        "random": "Math.random() is refused in eval (random)",
        "now": "Date.now() is refused in eval (clock)",
        "date": "new Date() is refused in eval (clock)",
        "performance": "performance.now() is refused in eval (clock)",
        "timers": ["undefined", "undefined", "undefined", "undefined"],
    }));
}

/// The timers are absent, `AbortSignal.timeout` refuses, and `Atomics.wait`
/// can't block the main thread.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn no_timer_is_reachable() {
    eval(&["timers.js"]).assert_value(&json!({
        "globals": ["undefined", "undefined", "undefined", "undefined"],
        "abort_signal_timeout": format!(
            "Error: {}",
            refusal("AbortSignal.timeout()", "timer")
        ),
        "atomics_wait": "TypeError: Atomics.wait cannot be called in this context",
    }));
}

/// A time zone other than UTC fails the eval before any code runs, naming
/// the fix; `Etc/UTC`, which ICU resolves to `UTC`, is accepted.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn a_time_zone_other_than_utc_is_refused() {
    let run = eval_with(&["value.js"], None, &[("TZ", "America/New_York")]);
    run.assert_failed("environment", 1);
    assert_eq!(
        run.message(),
        "eval runs only in the time zone UTC, so dates read and format the same everywhere, and \
         this process's is America/New_York: run fgro with TZ=UTC in its environment"
    );
    assert_eq!(run.envelope["access"], json!({"dynamic": []}));
    // before the driver runs, too
    let run = eval_with(
        &["value.js", "--driver", "-"],
        Some("throws.js"),
        &[("TZ", "America/New_York")],
    );
    run.assert_failed("environment", 1);
    eval_with(&["typed.ts"], None, &[("TZ", "Etc/UTC")]).assert_value(&json!({"x": 1, "y": 2}));
}

/// The zone must display as UTC too: an alias `Intl` resolves to `UTC` but
/// dates name `Greenwich Mean Time` is refused, naming that, while the
/// aliases that print exactly as `UTC` are accepted. A zone ICU doesn't know
/// (which `Intl` reports as no zone, or `Etc/Unknown` for an empty `TZ`) is
/// refused as unknown.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn a_utc_alias_with_another_display_name_is_refused() {
    let refused = |found: &str| {
        format!(
            "eval runs only in the time zone UTC, so dates read and format the same everywhere, \
             and {found}: run fgro with TZ=UTC in its environment"
        )
    };
    for zone in [
        "Etc/GMT",
        "GMT0",
        "Etc/GMT0",
        "Etc/GMT+0",
        "Etc/GMT-0",
        "Greenwich",
    ] {
        let run = eval_with(&["value.js"], None, &[("TZ", zone)]);
        run.assert_failed("environment", 1);
        assert_eq!(
            run.message(),
            refused("this process's is an alias of UTC that dates name Greenwich Mean Time"),
            "{zone}"
        );
    }
    for zone in ["UTC0", "XYZ", "/nonexistent/zone", ""] {
        let run = eval_with(&["value.js"], None, &[("TZ", zone)]);
        run.assert_failed("environment", 1);
        assert_eq!(
            run.message(),
            refused("this process's time zone isn't one ICU knows"),
            "{zone:?}"
        );
    }
    for zone in [
        "UTC",
        "Etc/UTC",
        "Universal",
        "Zulu",
        "UCT",
        "Etc/UCT",
        "Etc/Universal",
        "Etc/Zulu",
        ":UTC",
        ":Etc/UTC",
    ] {
        eval_with(&["typed.ts"], None, &[("TZ", zone)]).assert_value(&json!({"x": 1, "y": 2}));
    }
}

/// A zone that isn't UTC is refused even where its offset is zero, and so
/// is an unset `TZ`, which leaves ICU the host's zone (here one laid over
/// `/etc/localtime` in a private mount namespace): dates name it otherwise.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn a_zone_at_utcs_offset_or_an_unset_tz_is_refused() {
    let refused = |found: &str| {
        format!(
            "eval runs only in the time zone UTC, so dates read and format the same everywhere, \
             and this process's is {found}: run fgro with TZ=UTC in its environment"
        )
    };
    for (zone, found) in [
        ("Africa/Abidjan", "Africa/Abidjan"),
        ("Iceland", "Atlantic/Reykjavik"),
        ("Factory", "Factory"),
        // ICU's id for `GMT` names its offset
        ("GMT", "+00:00"),
    ] {
        let run = eval_with(&["value.js"], None, &[("TZ", zone)]);
        run.assert_failed("environment", 1);
        assert_eq!(run.message(), refused(found), "{zone}");
    }
    // with `TZ` unset, ICU takes the zone `/etc/localtime` links to, the
    // host's; a private mount namespace lays a link to `Asia/Tokyo` over it,
    // so a host whose zone is UTC, as CI's is, still sees the refusal (which
    // needs unprivileged user and mount namespaces, as `cargo xtask build
    // --verify` does)
    let scratch = Scratch::new("unset_tz", &[("etc/localtime", "")]);
    let etc = scratch.0.join("etc");
    std::fs::remove_file(etc.join("localtime")).unwrap();
    std::os::unix::fs::symlink("/usr/share/zoneinfo/Asia/Tokyo", etc.join("localtime")).unwrap();
    let mut unshare = Command::new("unshare");
    unshare
        .args([
            "-rm",
            "sh",
            "-c",
            r#"mount -t overlay overlay -o "lowerdir=$1:/etc" /etc && shift && exec "$@""#,
            "sh",
        ])
        .arg(&etc);
    let mut command = wrapped_command(unshare, &["value.js"]);
    command.env_remove("TZ");
    let run = parse(spawn(command, None));
    run.assert_failed("environment", 1);
    assert_eq!(run.message(), refused("Asia/Tokyo"));
}
