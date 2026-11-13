//! The untimed correctness run's checks: what a runtime's stdout must hold
//! ([`Stdout`]), the value it gives, and where two values first differ, for
//! a suite's `parity`.

use serde_json::Value;

use crate::suite::Stdout;

/// The value `stdout` holds by `check`: an envelope's `value`, which needs
/// `"ok": true`.
///
/// # Errors
///
/// Returns why `stdout` isn't of that shape, for the run's error.
pub fn stdout_value(check: Stdout, stdout: &[u8]) -> Result<Value, String> {
    let parsed = parse_deep(stdout)?;
    match check {
        Stdout::Envelope => {
            if parsed.get("ok") != Some(&Value::Bool(true)) {
                let error = parsed.get("error").map_or_else(
                    || "no error".to_owned(),
                    |error| truncated(&error.to_string()),
                );
                return Err(format!("its envelope isn't `\"ok\": true`: {error}"));
            }
            // moved out, not cloned: a clone recurses as deep as the value
            let value = match parsed {
                Value::Object(mut envelope) => envelope.remove("value"),
                _ => None,
            };
            value.ok_or_else(|| "its envelope has no `value`".to_owned())
        }
    }
}

/// The deepest nesting of arrays and objects a stdout may hold: eval's
/// serializer refuses a value past 1000, and its envelope adds one, so
/// this leaves room while keeping the parse's recursion bounded.
const MAX_DEPTH: usize = 2048;

/// The stack the parse runs on, ample for [`MAX_DEPTH`] levels of
/// `serde_json`'s recursion in a debug build.
const PARSE_STACK: usize = 64 << 20;

/// Parses `stdout` as one JSON value nested at most [`MAX_DEPTH`] deep,
/// past `serde_json`'s default limit of 128, on a thread with a stack sized
/// for it; the depth is checked first, by a scan, so the recursion stays
/// bounded whatever the bytes.
fn parse_deep(stdout: &[u8]) -> Result<Value, String> {
    let depth = nesting_depth(stdout);
    if depth > MAX_DEPTH {
        return Err(format!(
            "its stdout nests {depth} deep, past the {MAX_DEPTH} the check reads"
        ));
    }
    let parse = move || {
        let mut deserializer = serde_json::Deserializer::from_slice(stdout);
        deserializer.disable_recursion_limit();
        let value = <Value as serde::Deserialize>::deserialize(&mut deserializer)?;
        deserializer.end()?;
        Ok::<Value, serde_json::Error>(value)
    };
    let parsed = std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("gro_bench_parse".to_owned())
            .stack_size(PARSE_STACK)
            .spawn_scoped(scope, parse)
            .map(|parsing| {
                parsing
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
    })
    .map_err(|error| format!("its stdout couldn't be parsed: no thread ({error})"))?;
    parsed.map_err(|error| format!("its stdout isn't one JSON value ({error})"))
}

/// The deepest nesting of arrays and objects in `bytes` read as JSON,
/// brackets inside strings skipped; a malformed text gets some count, which
/// the parse then refuses.
fn nesting_depth(bytes: &[u8]) -> usize {
    let (mut depth, mut deepest) = (0_usize, 0_usize);
    let (mut in_string, mut escaped) = (false, false);
    for &byte in bytes {
        if in_string {
            match byte {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'[' | b'{' => {
                depth += 1;
                deepest = deepest.max(depth);
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    deepest
}

/// Where two values first differ, and what each holds there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Difference {
    /// The path from `$` (`$["resources"][2]["id"]`), with the lengths when
    /// two arrays differ only in length, cut like [`truncated`].
    pub at: String,
    /// The first value's part at `at`, as compact JSON cut to
    /// [`VALUE_MAX`] characters, or `absent`.
    pub a: String,
    /// The second value's, the same way.
    pub b: String,
}

/// The characters of a value a [`Difference`] keeps, so the error naming
/// both stays one readable line.
pub const VALUE_MAX: usize = 80;

/// Where `a` and `b` first differ, or `None` when they're equal. Objects
/// compare as maps, whatever their key order.
pub fn first_difference(a: &Value, b: &Value) -> Option<Difference> {
    difference_at(a, b, "$")
}

fn difference_at(a: &Value, b: &Value, at: &str) -> Option<Difference> {
    match (a, b) {
        (Value::Array(xs), Value::Array(ys)) => {
            if let Some((index, (x, y))) = xs.iter().zip(ys).enumerate().find(|(_, (x, y))| x != y)
            {
                return difference_at(x, y, &format!("{at}[{index}]"));
            }
            (xs.len() != ys.len()).then(|| {
                differ(
                    &format!("{at} (lengths {} and {})", xs.len(), ys.len()),
                    Some(a),
                    Some(b),
                )
            })
        }
        (Value::Object(xs), Value::Object(ys)) => {
            let keys = xs
                .keys()
                .chain(ys.keys().filter(|key| !xs.contains_key(*key)));
            for key in keys {
                let at = format!("{at}[{}]", Value::String(key.clone()));
                match (xs.get(key), ys.get(key)) {
                    (Some(x), Some(y)) if x == y => {}
                    (Some(x), Some(y)) => return difference_at(x, y, &at),
                    (x, y) => return Some(differ(&format!("{at} (present in one only)"), x, y)),
                }
            }
            None
        }
        _ => (a != b).then(|| differ(at, Some(a), Some(b))),
    }
}

/// A [`Difference`] at `at`, each value rendered as JSON, which escapes
/// every control character, so a value can't break the line, and each part
/// cut, so a long key or deep path can't make it unbounded.
fn differ(at: &str, a: Option<&Value>, b: Option<&Value>) -> Difference {
    let render = |value: Option<&Value>| {
        value.map_or_else(
            || "absent".to_owned(),
            |value| truncated_to(&value.to_string(), VALUE_MAX),
        )
    };
    Difference {
        at: truncated(at),
        a: render(a),
        b: render(b),
    }
}

/// `text`, cut to a length an error line can carry.
pub fn truncated(text: &str) -> String {
    truncated_to(text, 400)
}

/// `text`, cut to `max` characters, with `…` when cut.
fn truncated_to(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn reads_the_value_by_the_check() {
        let envelope = br#"{"ok": true, "value": {"a": [1]}, "access": {}}"#;
        assert_eq!(
            stdout_value(Stdout::Envelope, envelope),
            Ok(json!({"a": [1]}))
        );
        let failed = br#"{"ok": false, "error": {"kind": "resolve"}}"#;
        assert!(
            stdout_value(Stdout::Envelope, failed)
                .unwrap_err()
                .contains("\"kind\":\"resolve\"")
        );
        assert!(stdout_value(Stdout::Envelope, br#"{"ok": true}"#).is_err());
        assert!(stdout_value(Stdout::Envelope, b"{} {}").is_err());
        assert!(stdout_value(Stdout::Envelope, b"").is_err());
    }

    #[test]
    fn reads_a_value_nested_past_serde_json_s_default_limit() {
        let nested = |depth: usize| "[".repeat(depth) + &"]".repeat(depth);
        // as deep as eval's serializer writes, inside its envelope
        let envelope = format!(r#"{{"ok": true, "value": {}}}"#, nested(1000));
        let value = stdout_value(Stdout::Envelope, envelope.as_bytes()).unwrap();
        let mut depth = 0;
        let mut at = &value;
        while let Some(inner) = at.as_array().and_then(|array| array.first()) {
            depth += 1;
            at = inner;
        }
        assert_eq!(depth, 999, "the innermost array is empty");
        // brackets in strings aren't nesting
        let envelope = |value: &str| format!(r#"{{"ok": true, "value": {value}}}"#);
        let text = envelope(&format!(r#"["{}"]"#, "[{".repeat(5000)));
        assert!(stdout_value(Stdout::Envelope, text.as_bytes()).is_ok());
        // the envelope's own object is one level
        let deepest = envelope(&nested(MAX_DEPTH - 1));
        assert!(stdout_value(Stdout::Envelope, deepest.as_bytes()).is_ok());
        let past = envelope(&nested(MAX_DEPTH));
        let error = stdout_value(Stdout::Envelope, past.as_bytes()).unwrap_err();
        assert!(error.contains("nests 2049 deep"), "{error}");
    }

    #[test]
    fn names_where_two_values_first_differ() {
        let at = |a: &Value, b: &Value| first_difference(a, b).map(|difference| difference.at);
        let a = json!({"resources": [{"id": "x", "type": "file"}], "target": {"host": "h"}});
        assert_eq!(first_difference(&a, &a.clone()), None);
        // key order is no difference
        let reordered =
            json!({"target": {"host": "h"}, "resources": [{"type": "file", "id": "x"}]});
        assert_eq!(first_difference(&a, &reordered), None);
        let changed = json!({"resources": [{"id": "y", "type": "file"}], "target": {"host": "h"}});
        assert_eq!(
            first_difference(&a, &changed),
            Some(Difference {
                at: r#"$["resources"][0]["id"]"#.to_owned(),
                a: r#""x""#.to_owned(),
                b: r#""y""#.to_owned(),
            })
        );
        let longer =
            json!({"resources": [{"id": "x", "type": "file"}, 1], "target": {"host": "h"}});
        assert_eq!(
            at(&a, &longer).as_deref(),
            Some(r#"$["resources"] (lengths 1 and 2)"#)
        );
        let missing = json!({"resources": [{"id": "x", "type": "file"}]});
        let difference = first_difference(&a, &missing).unwrap();
        assert_eq!(difference.at, r#"$["target"] (present in one only)"#);
        assert_eq!(
            (difference.a.as_str(), difference.b.as_str()),
            (r#"{"host":"h"}"#, "absent")
        );
        assert_eq!(at(&json!(1), &json!(1.0)).as_deref(), Some("$"));
    }

    #[test]
    fn a_difference_keeps_each_value_to_one_bounded_line() {
        let long = "\n".repeat(200) + "line\nbreak\u{1b}[31m";
        let difference = first_difference(&json!(long), &json!(1)).unwrap();
        assert!(!difference.a.contains(['\n', '\u{1b}']), "{}", difference.a);
        assert_eq!(difference.a.chars().count(), VALUE_MAX + 1);
        assert!(difference.a.ends_with('…'));
        assert_eq!(difference.b, "1");
        // a long key, escaped and cut the same way
        let key = "k\n".repeat(1000);
        let difference = first_difference(&json!({ &key: 1 }), &json!({ &key: 2 })).unwrap();
        assert!(!difference.at.contains('\n'), "{}", difference.at);
        assert!(difference.at.ends_with('…'));
        assert_eq!(difference.at.chars().count(), 401);
    }
}
