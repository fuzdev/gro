//! The strict host serializer: a V8 value walked in Rust, written as compact
//! JSON, or refused with the JS path to what isn't data.
//!
//! The walk reads V8's own state through its API and never the realm's
//! `JSON`, `Object`, or any other global or prototype method, which code in
//! the run can replace, and it never runs JS: it reads each property's
//! descriptor, so a getter is refused, never called, and `toJSON` is never
//! looked up. [`to_json`] walks inside V8's `DisallowJavascriptExecutionScope`,
//! so a path that would run JS throws instead, a host bug reported as such.
//!
//! What it writes:
//!
//! - `null`, `true`, `false`, and strings, escaped as `JSON.stringify`
//!   escapes them
//! - finite numbers, as JS's `Number::toString` writes them (V8's own), so
//!   `-0` is `0`
//! - an array whose prototype is `Array.prototype`: every index from `0` to
//!   its length; an array's other properties are left out, as
//!   `JSON.stringify` leaves them
//! - a plain object, whose prototype is `Object.prototype` or null: its own
//!   enumerable string-keyed properties, integer-like keys first in
//!   ascending order, then the rest in insertion order (V8's own key order,
//!   as `JSON.stringify` reads it), a member holding `undefined` left out
//! - an object of a kind V8 doesn't name (a `WeakRef`, an iterator, an
//!   `Intl` formatter, a web API object) whose prototype was set to
//!   `Object.prototype` or null is written as a plain object: its own
//!   enumerable data properties, never its internal state
//!
//! What it refuses, with the JS path (`$` the root, then `.name`, `["a b"]`,
//! and `[2]`):
//!
//! - a getter or setter, never run: on an enumerable member, or at an array
//!   index; non-enumerable members are skipped as `JSON.stringify` skips
//!   them, so their getters don't run either
//! - `undefined` at the root or in an array, and a hole in an array
//! - a function, a bigint, a symbol, `NaN`, and the infinities
//! - an object that isn't plain: any other prototype (a class instance),
//!   and an array with another prototype; and, whatever its prototype, a
//!   `Map`, `Date`, `RegExp`, `Promise`, error, boxed primitive, typed
//!   array, buffer, `arguments`, module namespace, or `Proxy`, or a host
//!   object
//! - a cycle: a value that one of its own members leads back to; a value
//!   reached twice by different paths is written twice
//! - a string or key holding a lone surrogate, which no UTF-8 reader can
//!   decode (`JSON.stringify` writes it as a `\udXXX` escape that Rust's and
//!   most readers' parsers refuse)
//! - nesting deeper than [`MAX_DEPTH`] arrays and objects
//!
//! The walk keeps its own stack of the arrays and objects it's inside of,
//! so the host's native stack stays the same depth however deep the value
//! nests, and it reads in batches, each in a handle scope of its own, so
//! what it holds is its output and its stack, whatever the value's size.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::fmt::{self, Write as _};
use std::num::NonZeroI32;

use deno_core::v8;

/// How many arrays and objects deep a value may nest, the root counting as
/// one.
pub const MAX_DEPTH: usize = 1000;

/// A value that didn't serialize: where, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerializeError {
    /// The JS path to the value refused, `$` for the root.
    pub at: String,
    /// Why it was refused.
    pub reason: Reason,
}

/// Why a value didn't serialize.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// An enumerable accessor property, whose getter isn't run.
    Accessor,
    /// `undefined` where a value must be: the root, or an array's item.
    Undefined,
    /// A hole in an array.
    Hole,
    /// A function.
    Function,
    /// A bigint.
    BigInt,
    /// A symbol.
    Symbol,
    /// `NaN`, `Infinity`, or `-Infinity`, as JS writes it.
    NonFinite(String),
    /// An object that isn't a plain object or an array, as named.
    NonPlain(String),
    /// A value one of its own members leads back to.
    Cycle,
    /// Arrays and objects nested deeper than [`MAX_DEPTH`].
    TooDeep,
    /// A lone surrogate code unit, in a string or, when `in_key`, a key.
    LoneSurrogate {
        /// The code unit.
        unit: u16,
        /// Whether it's in a key rather than a string value.
        in_key: bool,
    },
    /// V8 refused a read the walk made, which only a host bug can cause:
    /// every read is one that runs no JS.
    Refused,
}

impl SerializeError {
    /// Whether the host failed, rather than the value.
    pub const fn is_internal(&self) -> bool {
        matches!(self.reason, Reason::Refused)
    }
}

impl fmt::Display for SerializeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let at = &self.at;
        match &self.reason {
            Reason::Accessor => write!(
                f,
                "{at} is an accessor property (a getter or setter), and no getter is run; \
                 write the value as a data property"
            ),
            Reason::Undefined if at == "$" => {
                write!(f, "{at} is undefined, and the value must be JSON data")
            }
            Reason::Undefined => write!(
                f,
                "{at} is undefined; an array holds a value at every position, so leave the \
                 item out or write null"
            ),
            Reason::Hole => write!(
                f,
                "{at} is a hole in a sparse array; an array holds a value at every position"
            ),
            Reason::Function => write!(
                f,
                "{at} is a function; the value is data, so call it or write what it returns"
            ),
            Reason::BigInt => write!(f, "{at} is a bigint; write it as a number or a string"),
            Reason::Symbol => write!(f, "{at} is a symbol, which JSON can't write"),
            Reason::NonFinite(number) => write!(f, "{at} is {number}, which JSON can't write"),
            Reason::NonPlain(kind) => write!(
                f,
                "{at} is {kind}, not a plain object or an array; write what it holds as a plain \
                 object, an array, or a string"
            ),
            Reason::Cycle => write!(
                f,
                "{at} holds itself: one of its members leads back to it, which JSON can't write"
            ),
            Reason::TooDeep => write!(
                f,
                "{at} is nested deeper than {MAX_DEPTH} arrays and objects"
            ),
            Reason::LoneSurrogate { unit, in_key } => write!(
                f,
                "{at} holds a lone surrogate, U+{unit:04X}, in {}, which isn't Unicode text and \
                 can't be read as UTF-8",
                if *in_key { "a key" } else { "its text" }
            ),
            Reason::Refused => write!(
                f,
                "V8 refused to read {at} without running JS, a host bug: the serializer reads \
                 only what runs no JS"
            ),
        }
    }
}

/// How many walk steps run in one handle scope: a step reads one member, or
/// closes an array or object, so the handles one batch makes (a key, a
/// descriptor, a value, each frame's object) are freed when it ends, and a
/// walk's memory is its output and its frames, whatever the value's size.
const BATCH: usize = 1024;

/// `value` as compact JSON text, by the rules in the module doc.
///
/// `scope` must have a context entered; the walk runs no JS in it.
///
/// # Errors
///
/// Returns where and why `value` isn't JSON data.
pub fn to_json(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Result<String, SerializeError> {
    v8::tc_scope!(let tc, scope);
    v8::disallow_javascript_execution_scope!(let scope, tc, v8::OnFailure::ThrowOnFailure);
    let value = v8::Local::new(scope, value);
    // the realm's intrinsics, which no code in the run can replace: a fresh
    // literal's prototype is %Object.prototype%, whatever `globalThis.Object` is
    let object_prototype = v8::Object::new(scope).get_prototype(scope);
    let array_prototype = v8::Array::new(scope, 0).get_prototype(scope);
    let value_key = v8::String::new(scope, "value");
    let (Some(object_prototype), Some(array_prototype), Some(value_key)) =
        (object_prototype, array_prototype, value_key)
    else {
        return Err(SerializeError {
            at: "$".to_owned(),
            reason: Reason::Refused,
        });
    };
    let mut walker = Walker {
        out: String::new(),
        path: Vec::new(),
        frames: Vec::new(),
        within: HashMap::new(),
        object_prototype: v8::Global::new(scope, object_prototype),
        array_prototype: v8::Global::new(scope, array_prototype),
        value_key: v8::Global::new(scope, value_key),
    };
    walker.visit(scope, value)?;
    while !walker.frames.is_empty() {
        // once per batch: where a deadline, a heap limit, or an output
        // budget is checked
        walker.batch(scope)?;
    }
    Ok(walker.out)
}

/// A step of the path to a value.
#[derive(Debug)]
enum Segment {
    /// An array's index.
    Index(u32),
    /// An object's key, as UTF-16 code units.
    Key(Vec<u16>),
}

/// An array or object the walk is inside of, written up to `next`.
struct Frame {
    /// The array or object.
    object: v8::Global<v8::Object>,
    /// Its identity hash, the key of [`Walker::within`].
    hash: NonZeroI32,
    /// What its members are named by.
    members: Members,
    /// How many members it has.
    length: u32,
    /// The member to read next.
    next: u32,
    /// Whether a member is written yet, for an object, which leaves members
    /// out: the first written takes no comma.
    wrote: bool,
}

/// How a frame's members are named.
enum Members {
    /// An array's, by index from `0`.
    Indices,
    /// An object's, by the keys V8 listed, in its order.
    Keys(v8::Global<v8::Array>),
}

/// The walk's state: an explicit stack of frames, so the host's native
/// stack stays the same depth however deep the value nests.
struct Walker {
    /// The JSON written so far.
    out: String,
    /// The path to the value being walked: one segment per frame below the
    /// root's, plus the member being read.
    path: Vec<Segment>,
    /// The arrays and objects the walk is inside of, outermost first.
    frames: Vec<Frame>,
    /// How many frames hold an object of each identity hash, so a cycle is
    /// found without scanning the frames, except on a hash collision.
    within: HashMap<NonZeroI32, u32>,
    /// `%Object.prototype%`.
    object_prototype: v8::Global<v8::Value>,
    /// `%Array.prototype%`.
    array_prototype: v8::Global<v8::Value>,
    /// `"value"`, the key of a data descriptor's value.
    value_key: v8::Global<v8::String>,
}

impl Walker {
    /// The path to the value being walked, as JS writes it.
    fn at(&self) -> String {
        let mut at = String::from("$");
        for segment in &self.path {
            match segment {
                Segment::Index(index) => {
                    at.push('[');
                    at.push_str(&index.to_string());
                    at.push(']');
                }
                Segment::Key(units) => {
                    let key = String::from_utf16_lossy(units);
                    if is_identifier(&key) {
                        at.push('.');
                        at.push_str(&key);
                    } else {
                        at.push('[');
                        push_json_string(&mut at, &key);
                        at.push(']');
                    }
                }
            }
        }
        at
    }

    /// An error at the value being walked.
    fn error(&self, reason: Reason) -> SerializeError {
        SerializeError {
            at: self.at(),
            reason,
        }
    }

    /// Runs up to [`BATCH`] steps in a handle scope of their own.
    fn batch(&mut self, scope: &mut v8::PinScope<'_, '_>) -> Result<(), SerializeError> {
        v8::scope!(let scope, scope);
        for _ in 0..BATCH {
            if self.frames.is_empty() {
                break;
            }
            self.step(scope)?;
        }
        Ok(())
    }

    /// Reads the innermost frame's next member, or closes the frame when it
    /// has none left.
    fn step(&mut self, scope: &v8::PinScope<'_, '_>) -> Result<(), SerializeError> {
        let Some(frame) = self.frames.last_mut() else {
            return Ok(());
        };
        let index = frame.next;
        if index == frame.length {
            let close = match frame.members {
                Members::Indices => ']',
                Members::Keys(_) => '}',
            };
            self.out.push(close);
            self.leave();
            return Ok(());
        }
        frame.next += 1;
        let object = v8::Local::new(scope, &frame.object);
        let keys = match &frame.members {
            Members::Indices => None,
            Members::Keys(keys) => Some(v8::Local::new(scope, keys)),
        };
        let Some(keys) = keys else {
            if index > 0 {
                self.out.push(',');
            }
            self.path.push(Segment::Index(index));
            // `ToString` of a primitive number, which runs no JS
            let key = v8::Integer::new_from_unsigned(scope, index)
                .to_string(scope)
                .ok_or_else(|| self.error(Reason::Refused))?;
            let item = self
                .own_data(scope, object, key)?
                .ok_or_else(|| self.error(Reason::Hole))?;
            if !self.visit(scope, item)? {
                self.path.pop();
            }
            return Ok(());
        };
        // an array V8 made, read by index from its own elements
        let key = keys
            .get_index(scope, index)
            .filter(|key| key.is_string())
            .ok_or_else(|| self.error(Reason::Refused))?;
        let key: v8::Local<'_, v8::String> = key.cast();
        self.path.push(Segment::Key(read_string(scope, key)));
        let Some(item) = self.own_data(scope, object, key)? else {
            // deleted since the keys were read, which no JS here can do
            return Err(self.error(Reason::Refused));
        };
        if item.is_undefined() {
            self.path.pop();
            return Ok(());
        }
        if let Some(frame) = self.frames.last_mut() {
            if frame.wrote {
                self.out.push(',');
            }
            frame.wrote = true;
        }
        let written = match self.path.last() {
            Some(Segment::Key(units)) => push_utf16_json_string(&mut self.out, units),
            _ => Ok(()),
        };
        written.map_err(|unit| self.error(Reason::LoneSurrogate { unit, in_key: true }))?;
        self.out.push(':');
        if !self.visit(scope, item)? {
            self.path.pop();
        }
        Ok(())
    }

    /// Closes the innermost frame: the walk leaves it, and the path the
    /// segment that led into it.
    fn leave(&mut self) {
        if let Some(frame) = self.frames.pop()
            && let Entry::Occupied(mut count) = self.within.entry(frame.hash)
        {
            *count.get_mut() -= 1;
            if *count.get() == 0 {
                count.remove();
            }
        }
        // the root's frame has no segment
        if !self.frames.is_empty() {
            self.path.pop();
        }
    }

    /// Writes `value` when it's a primitive, or opens its frame when it's an
    /// array or plain object, returning whether it opened one.
    fn visit(
        &mut self,
        scope: &v8::PinScope<'_, '_>,
        value: v8::Local<'_, v8::Value>,
    ) -> Result<bool, SerializeError> {
        if value.is_null() {
            self.out.push_str("null");
        } else if value.is_true() {
            self.out.push_str("true");
        } else if value.is_false() {
            self.out.push_str("false");
        } else if value.is_number() {
            // `ToString` of a primitive number, which runs no JS
            let text = value
                .to_string(scope)
                .ok_or_else(|| self.error(Reason::Refused))?;
            let text = String::from_utf16_lossy(&read_string(scope, text));
            if value.cast::<v8::Number>().value().is_finite() {
                self.out.push_str(&text);
            } else {
                return Err(self.error(Reason::NonFinite(text)));
            }
        } else if value.is_string() {
            let text = read_string(scope, value.cast());
            push_utf16_json_string(&mut self.out, &text).map_err(|unit| {
                self.error(Reason::LoneSurrogate {
                    unit,
                    in_key: false,
                })
            })?;
        } else if value.is_undefined() {
            return Err(self.error(Reason::Undefined));
        } else if value.is_big_int() {
            return Err(self.error(Reason::BigInt));
        } else if value.is_symbol() {
            return Err(self.error(Reason::Symbol));
        } else if value.is_function() {
            return Err(self.error(Reason::Function));
        } else if value.is_object() {
            self.enter(scope, value)?;
            return Ok(true);
        } else {
            // every JS value is one of the above
            return Err(self.error(Reason::Refused));
        }
        Ok(false)
    }

    /// Opens the frame of the array or plain object `value`, writing its
    /// opening bracket, or refuses it.
    fn enter(
        &mut self,
        scope: &v8::PinScope<'_, '_>,
        value: v8::Local<'_, v8::Value>,
    ) -> Result<(), SerializeError> {
        // an array is never exotic: V8 names only a true array one, never a
        // `Proxy` of one
        let is_array = value.is_array();
        if !is_array && let Some(kind) = exotic_kind(value) {
            return Err(self.error(Reason::NonPlain(kind.to_owned())));
        }
        let object: v8::Local<'_, v8::Object> = value.cast();
        let hash = object.get_identity_hash();
        if self.within.contains_key(&hash)
            && self
                .frames
                .iter()
                .any(|frame| frame.hash == hash && frame.object == object)
        {
            return Err(self.error(Reason::Cycle));
        }
        if self.frames.len() >= MAX_DEPTH {
            return Err(self.error(Reason::TooDeep));
        }
        // no `Proxy` gets here, so reading the prototype runs no trap
        let prototype = object.get_prototype(scope);
        let (members, length) = if is_array {
            if !prototype.is_some_and(|prototype| self.array_prototype == prototype) {
                return Err(self.error(Reason::NonPlain(
                    "an array whose prototype is not `Array.prototype`".to_owned(),
                )));
            }
            self.out.push('[');
            // the length V8 holds, read without a getter
            (Members::Indices, object.cast::<v8::Array>().length())
        } else {
            let plain = prototype
                .is_some_and(|prototype| prototype.is_null() || self.object_prototype == prototype);
            if !plain {
                // read from the object's map, running no JS
                let name =
                    String::from_utf16_lossy(&read_string(scope, object.get_constructor_name()));
                let mut kind =
                    "an object whose prototype is neither `Object.prototype` nor null".to_owned();
                if name != "Object" {
                    let _ = write!(kind, " (its constructor is `{name}`)");
                }
                return Err(self.error(Reason::NonPlain(kind)));
            }
            if object.internal_field_count() > 0 {
                return Err(self.error(Reason::NonPlain("a host object".to_owned())));
            }
            let keys = object
                .get_own_property_names(
                    scope,
                    v8::GetPropertyNamesArgs {
                        mode: v8::KeyCollectionMode::OwnOnly,
                        property_filter: v8::PropertyFilter::ONLY_ENUMERABLE
                            | v8::PropertyFilter::SKIP_SYMBOLS,
                        index_filter: v8::IndexFilter::IncludeIndices,
                        key_conversion: v8::KeyConversionMode::ConvertToString,
                    },
                )
                .ok_or_else(|| self.error(Reason::Refused))?;
            self.out.push('{');
            let length = keys.length();
            (Members::Keys(v8::Global::new(scope, keys)), length)
        };
        *self.within.entry(hash).or_default() += 1;
        self.frames.push(Frame {
            object: v8::Global::new(scope, object),
            hash,
            members,
            length,
            next: 0,
            wrote: false,
        });
        Ok(())
    }

    /// The value of `object`'s own data property `key`, read from its
    /// descriptor, or `None` when it has no own property of that name. An
    /// accessor is refused, its getter never called.
    fn own_data<'s>(
        &self,
        scope: &v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
        key: v8::Local<'s, v8::String>,
    ) -> Result<Option<v8::Local<'s, v8::Value>>, SerializeError> {
        let refused = || self.error(Reason::Refused);
        let descriptor = object
            .get_own_property_descriptor(scope, key.into())
            .ok_or_else(refused)?;
        if descriptor.is_undefined() {
            return Ok(None);
        }
        let descriptor: v8::Local<'s, v8::Object> = descriptor.try_cast().map_err(|_| refused())?;
        let value_key = v8::Local::new(scope, &self.value_key);
        // a data descriptor has its own `value`, an accessor's has `get` and
        // `set`; read as own data, never through `Object.prototype`
        if !descriptor
            .has_own_property(scope, value_key.into())
            .ok_or_else(refused)?
        {
            return Err(self.error(Reason::Accessor));
        }
        descriptor
            .get(scope, value_key.into())
            .map(Some)
            .ok_or_else(refused)
    }
}

/// The name of `value`'s kind when it's an exotic or host object that no
/// prototype makes plain, else `None`.
fn exotic_kind(value: v8::Local<'_, v8::Value>) -> Option<&'static str> {
    /// A predicate on a value, and the kind it names.
    type Kind = (fn(&v8::Value) -> bool, &'static str);
    let kinds: [Kind; 27] = [
        (v8::Value::is_proxy, "a `Proxy`"),
        (v8::Value::is_module_namespace_object, "a module namespace"),
        (v8::Value::is_map, "a `Map`"),
        (v8::Value::is_set, "a `Set`"),
        (v8::Value::is_weak_map, "a `WeakMap`"),
        (v8::Value::is_weak_set, "a `WeakSet`"),
        (v8::Value::is_map_iterator, "a `Map` iterator"),
        (v8::Value::is_set_iterator, "a `Set` iterator"),
        (v8::Value::is_date, "a `Date`"),
        (v8::Value::is_reg_exp, "a `RegExp`"),
        (v8::Value::is_promise, "a `Promise`"),
        (v8::Value::is_native_error, "an error"),
        (v8::Value::is_generator_object, "a generator"),
        (v8::Value::is_arguments_object, "an `arguments` object"),
        (v8::Value::is_string_object, "a `String` object"),
        (v8::Value::is_number_object, "a `Number` object"),
        (v8::Value::is_boolean_object, "a `Boolean` object"),
        (v8::Value::is_big_int_object, "a `BigInt` object"),
        (v8::Value::is_symbol_object, "a `Symbol` object"),
        (v8::Value::is_array_buffer, "an `ArrayBuffer`"),
        (v8::Value::is_shared_array_buffer, "a `SharedArrayBuffer`"),
        (v8::Value::is_typed_array, "a typed array"),
        (v8::Value::is_data_view, "a `DataView`"),
        (v8::Value::is_array_buffer_view, "a buffer view"),
        (v8::Value::is_external, "a host value"),
        (v8::Value::is_wasm_memory_object, "a `WebAssembly.Memory`"),
        (v8::Value::is_wasm_module_object, "a `WebAssembly.Module`"),
    ];
    kinds
        .iter()
        .find(|(is, _)| is(&value))
        .map(|(_, kind)| *kind)
}

/// The UTF-16 code units of `string`.
fn read_string(scope: &v8::PinScope<'_, '_>, string: v8::Local<'_, v8::String>) -> Vec<u16> {
    let mut units = vec![0; string.length()];
    string.write_v2(scope, 0, &mut units, v8::WriteFlags::empty());
    units
}

/// Whether `key` reads as an identifier in a path (`$.key`), else it's
/// written quoted (`$["a key"]`).
fn is_identifier(key: &str) -> bool {
    let mut chars = key.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_' || first == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

/// Appends `text` as a JSON string, escaped as `JSON.stringify` escapes:
/// `"` and `\`, the short forms `\b`, `\f`, `\n`, `\r`, and `\t`, and the
/// other control characters as `\u00xx`; everything else as itself.
fn push_json_string(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        push_json_char(out, c);
    }
    out.push('"');
}

/// [`push_json_string`] for UTF-16 code units, failing on the first lone
/// surrogate with its code unit, which leaves `out` partly written.
fn push_utf16_json_string(out: &mut String, units: &[u16]) -> Result<(), u16> {
    out.push('"');
    for c in char::decode_utf16(units.iter().copied()) {
        push_json_char(out, c.map_err(|error| error.unpaired_surrogate())?);
    }
    out.push('"');
    Ok(())
}

/// Appends `c` as it's written inside a JSON string.
fn push_json_char(out: &mut String, c: char) {
    match c {
        '"' => out.push_str("\\\""),
        '\\' => out.push_str("\\\\"),
        '\u{8}' => out.push_str("\\b"),
        '\u{c}' => out.push_str("\\f"),
        '\n' => out.push_str("\\n"),
        '\r' => out.push_str("\\r"),
        '\t' => out.push_str("\\t"),
        c if u32::from(c) < 0x20 => {
            let _ = write!(out, "\\u{:04x}", u32::from(c));
        }
        c => out.push(c),
    }
}

#[cfg(test)]
mod tests;
