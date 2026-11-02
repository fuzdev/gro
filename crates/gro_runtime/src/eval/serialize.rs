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
//!   `-0` is `0`; a safe integer is written by the walk itself, digit for
//!   digit what V8 writes, and any other number by V8
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
//! and `[2]`; a key longer than [`MAX_KEY_SHOWN`] UTF-16 units is cut to
//! them and named by its length, `["aaa…" (70000 units)]`, so a path stays
//! small whatever the keys):
//!
//! - a getter or setter, never run: on an enumerable member, or at an array
//!   index; non-enumerable members are skipped as `JSON.stringify` skips
//!   them, so their getters don't run either
//! - `undefined` at the root or in an array, and a hole in an array
//! - a function, a bigint, a symbol, `NaN`, and the infinities
//! - an object that isn't plain: any other prototype (a class instance),
//!   and an array with another prototype; and, whatever its prototype, a
//!   `Map`, `Set`, `Date`, `RegExp`, `Promise`, error, boxed primitive, typed
//!   array, buffer, `arguments`, module namespace, or `Proxy`, or a host
//!   object
//! - a cycle: a value that one of its own members leads back to; a value
//!   reached twice by different paths is written twice
//! - a string or key holding a lone surrogate, which no UTF-8 reader can
//!   decode (`JSON.stringify` writes it as a `\udXXX` escape that Rust's and
//!   most readers' parsers refuse)
//! - nesting deeper than [`MAX_DEPTH`] arrays and objects
//! - a value whose JSON would pass the `max` bytes the caller gives
//!
//! The walk keeps its own stack of the arrays and objects it's inside of,
//! so the host's native stack stays the same depth however deep the value
//! nests, and it reads in batches, each in a handle scope of its own, so
//! what it holds is its output and its stack, whatever the value's size.
//! Before each batch it asks whether the host has stopped the run (a
//! timeout, the heap's cap), since no terminate interrupts a Rust loop.
//! It stops as soon as its output passes `max`: a string's length is
//! checked before it's read (each UTF-16 unit writes at least one byte),
//! and the output as each character and each step is written, so the
//! output never holds much more than `max` bytes.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::fmt::{self, Write as _};
use std::num::NonZeroI32;

use deno_core::v8;

use super::limits::format_size;

/// How many arrays and objects deep a value may nest, the root counting as
/// one.
const MAX_DEPTH: usize = 1000;

/// The most UTF-16 units of a key that a path shows: a longer key is cut
/// to them, then named by its length, so a path is at most [`MAX_DEPTH`]
/// segments of a few hundred bytes each.
const MAX_KEY_SHOWN: usize = 64;

/// A value that didn't serialize: where, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SerializeError {
    /// The JS path to the value refused, `$` for the root, each key longer
    /// than [`MAX_KEY_SHOWN`] cut.
    pub(super) at: String,
    /// Why it was refused.
    reason: Reason,
}

/// Why a value didn't serialize.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Reason {
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
    /// An object that isn't a plain object or an array.
    NonPlain(NonPlain),
    /// A value one of its own members leads back to.
    Cycle,
    /// Arrays and objects nested deeper than [`MAX_DEPTH`].
    TooDeep,
    /// The JSON written would pass the caller's cap of `max` bytes.
    TooLarge {
        /// The cap, in bytes.
        max: usize,
    },
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
    /// The host stopped the run (a timeout, the heap's cap) during the
    /// walk, which the host reports as such.
    Stopped,
}

/// What makes an object not plain, which a refusal names.
#[derive(Debug, Clone, PartialEq, Eq)]
enum NonPlain {
    /// An exotic or host object that no prototype makes plain, by the name
    /// of its kind ([`exotic_kind`]): a `Map`, a `Proxy`, a typed array.
    Exotic(&'static str),
    /// An array whose prototype isn't `Array.prototype`.
    ArrayPrototype,
    /// An object whose prototype is neither `Object.prototype` nor null.
    Prototype {
        /// Its constructor's name, read from its map, unless it's `Object`.
        constructor: Option<String>,
    },
    /// An object with internal fields, which only the host makes.
    HostObject,
}

impl fmt::Display for NonPlain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exotic(kind) => f.write_str(kind),
            Self::ArrayPrototype => {
                f.write_str("an array whose prototype is not `Array.prototype`")
            }
            Self::Prototype { constructor } => {
                f.write_str("an object whose prototype is neither `Object.prototype` nor null")?;
                if let Some(name) = constructor {
                    write!(f, " (its constructor is `{name}`)")?;
                }
                Ok(())
            }
            Self::HostObject => f.write_str("a host object"),
        }
    }
}

impl SerializeError {
    /// Whether the host failed, rather than the value.
    pub(super) const fn is_internal(&self) -> bool {
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
            Reason::TooLarge { max } => write!(
                f,
                "{at} takes the value's JSON past {}, the envelope's cap; write less data",
                format_size(*max as u64)
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
            Reason::Stopped => write!(f, "the run was stopped while {at} was serialized"),
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
/// `stopped` is asked before each batch whether the host has stopped the
/// run, which ends the walk. The JSON is at most `max` bytes, and the walk
/// ends as soon as it would pass them.
///
/// # Errors
///
/// Returns where and why `value` isn't JSON data, where the walk was when
/// `stopped` said so, or where the JSON passed `max` bytes.
pub(super) fn to_json(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    stopped: &dyn Fn() -> bool,
    max: usize,
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
        max,
        path: Vec::new(),
        frames: Vec::new(),
        within: HashMap::new(),
        object_prototype: v8::Global::new(scope, object_prototype),
        array_prototype: v8::Global::new(scope, array_prototype),
        value_key: v8::Global::new(scope, value_key),
        units: Vec::new(),
    };
    walker.visit(scope, value)?;
    while !walker.frames.is_empty() {
        // once per batch: a terminate, the timeout's or the heap's, can't
        // interrupt this loop, which runs no JS
        if stopped() {
            return Err(walker.error(Reason::Stopped));
        }
        walker.batch(scope)?;
    }
    Ok(walker.out)
}

/// A step of the path to a value.
#[derive(Debug)]
enum Segment {
    /// An array's index.
    Index(u32),
    /// An object's key, as much as a path shows, held inline, so a member
    /// costs no allocation for its segment.
    Key {
        /// Its first [`MAX_KEY_SHOWN`] UTF-16 code units at most, one fewer
        /// when the cut would split a surrogate pair, in `head[..shown]`.
        head: [u16; MAX_KEY_SHOWN],
        /// How many units of `head` are the key's.
        shown: usize,
        /// Its length in UTF-16 code units.
        length: usize,
    },
}

impl Segment {
    /// The segment of the key `units`, cut to what a path shows.
    fn key(units: &[u16]) -> Self {
        let mut end = units.len().min(MAX_KEY_SHOWN);
        // a high surrogate at the cut is half of a pair the cut splits
        if end < units.len() && (0xd800..0xdc00).contains(&units[end - 1]) {
            end -= 1;
        }
        let mut head = [0; MAX_KEY_SHOWN];
        head[..end].copy_from_slice(&units[..end]);
        Self::Key {
            head,
            shown: end,
            length: units.len(),
        }
    }
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
    /// The most bytes `out` may hold.
    max: usize,
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
    /// The UTF-16 units of the string or key being written, reused across
    /// them, so a string costs no allocation of its own.
    units: Vec<u16>,
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
                Segment::Key {
                    head,
                    shown,
                    length,
                } if shown < length => {
                    let mut cut = String::from_utf16_lossy(&head[..*shown]);
                    cut.push('…');
                    at.push('[');
                    push_json_string(&mut at, &cut);
                    let _ = write!(at, " ({length} units)]");
                }
                Segment::Key { head, shown, .. } => {
                    let key = String::from_utf16_lossy(&head[..*shown]);
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

    /// Refuses the value once the output has passed its cap.
    fn check_size(&self) -> Result<(), SerializeError> {
        if self.out.len() > self.max {
            return Err(self.too_large());
        }
        Ok(())
    }

    /// Refuses `string` before it's read when writing it would pass the
    /// cap: each of its UTF-16 units writes at least one byte, and its
    /// quotes two.
    fn check_string(&self, string: v8::Local<'_, v8::String>) -> Result<(), SerializeError> {
        if self
            .out
            .len()
            .saturating_add(string.length())
            .saturating_add(2)
            > self.max
        {
            return Err(self.too_large());
        }
        Ok(())
    }

    /// The error for output past the cap, at the value being walked.
    fn too_large(&self) -> SerializeError {
        self.error(Reason::TooLarge { max: self.max })
    }

    /// The error for a string that stopped being written, in a key when
    /// `in_key`.
    fn string_error(&self, stop: StringStop, in_key: bool) -> SerializeError {
        match stop {
            StringStop::LoneSurrogate(unit) => self.error(Reason::LoneSurrogate { unit, in_key }),
            StringStop::TooLarge => self.too_large(),
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
            // a comma or a closing bracket; the rest is checked as it's written
            self.check_size()?;
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
            let item = Self::own_data(scope, &self.value_key, object, key)
                .map_err(|reason| self.error(reason))?
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
        // read before the key, so a member never written costs nothing
        let item = Self::own_data(scope, &self.value_key, object, key);
        if let Ok(Some(item)) = item
            && item.is_undefined()
        {
            return Ok(());
        }
        self.check_string(key)?;
        read_string(scope, key, &mut self.units);
        self.path.push(Segment::key(&self.units));
        let Some(item) = item.map_err(|reason| self.error(reason))? else {
            // deleted since the keys were read, which no JS here can do
            return Err(self.error(Reason::Refused));
        };
        if let Some(frame) = self.frames.last_mut() {
            if frame.wrote {
                self.out.push(',');
            }
            frame.wrote = true;
        }
        push_utf16_json_string(&mut self.out, &self.units, self.max)
            .map_err(|stop| self.string_error(stop, true))?;
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
            let number = value.cast::<v8::Number>().value();
            if let Some(integer) = safe_integer(number) {
                // what `Number::toString` writes for it, digit for digit
                let _ = write!(self.out, "{integer}");
            } else {
                // `ToString` of a primitive number, which runs no JS
                let text = value
                    .to_string(scope)
                    .ok_or_else(|| self.error(Reason::Refused))?;
                read_string(scope, text, &mut self.units);
                if !number.is_finite() {
                    let text = String::from_utf16_lossy(&self.units);
                    return Err(self.error(Reason::NonFinite(text)));
                }
                // a number's text is ASCII
                self.out.extend(
                    char::decode_utf16(self.units.iter().copied())
                        .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER)),
                );
            }
        } else if value.is_string() {
            let string = value.cast();
            self.check_string(string)?;
            read_string(scope, string, &mut self.units);
            push_utf16_json_string(&mut self.out, &self.units, self.max)
                .map_err(|stop| self.string_error(stop, false))?;
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
            self.check_size()?;
            return Ok(true);
        } else {
            // every JS value is one of the above
            return Err(self.error(Reason::Refused));
        }
        // at the value, before the step leaves it
        self.check_size()?;
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
            return Err(self.error(Reason::NonPlain(NonPlain::Exotic(kind))));
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
                return Err(self.error(Reason::NonPlain(NonPlain::ArrayPrototype)));
            }
            self.out.push('[');
            // the length V8 holds, read without a getter
            (Members::Indices, object.cast::<v8::Array>().length())
        } else {
            let plain = prototype
                .is_some_and(|prototype| prototype.is_null() || self.object_prototype == prototype);
            if !plain {
                // read from the object's map, running no JS
                read_string(scope, object.get_constructor_name(), &mut self.units);
                let name = String::from_utf16_lossy(&self.units);
                let constructor = (name != "Object").then_some(name);
                return Err(self.error(Reason::NonPlain(NonPlain::Prototype { constructor })));
            }
            if object.internal_field_count() > 0 {
                return Err(self.error(Reason::NonPlain(NonPlain::HostObject)));
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
        scope: &v8::PinScope<'s, '_>,
        value_key: &v8::Global<v8::String>,
        object: v8::Local<'s, v8::Object>,
        key: v8::Local<'s, v8::String>,
    ) -> Result<Option<v8::Local<'s, v8::Value>>, Reason> {
        let descriptor = object
            .get_own_property_descriptor(scope, key.into())
            .ok_or(Reason::Refused)?;
        if descriptor.is_undefined() {
            return Ok(None);
        }
        let descriptor: v8::Local<'s, v8::Object> =
            descriptor.try_cast().map_err(|_| Reason::Refused)?;
        let value_key = v8::Local::new(scope, value_key);
        // a data descriptor has its own `value`, an accessor's has `get` and
        // `set`; read as own data, never through `Object.prototype`
        if !descriptor
            .has_own_property(scope, value_key.into())
            .ok_or(Reason::Refused)?
        {
            return Err(Reason::Accessor);
        }
        descriptor
            .get(scope, value_key.into())
            .map(Some)
            .ok_or(Reason::Refused)
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

/// Reads the UTF-16 code units of `string` into `units`, replacing what it
/// held.
fn read_string(
    scope: &v8::PinScope<'_, '_>,
    string: v8::Local<'_, v8::String>,
    units: &mut Vec<u16>,
) {
    units.clear();
    units.resize(string.length(), 0);
    string.write_v2(scope, 0, units, v8::WriteFlags::empty());
}

/// The largest integer below which every integer is a double, `2^53`.
const SAFE_INTEGER_BOUND: f64 = 9_007_199_254_740_992.0;

/// `number` as an integer when it is a safe one, `-0` as `0`, else `None`.
///
/// `Number::toString` writes a safe integer as its decimal digits, which
/// is what Rust writes for it: the spec takes the fewest significant
/// digits whose value rounds to the number, and below `2^53` doubles are
/// at most 1 apart, so a value with fewer significant digits than the
/// integer's own differs from it by at least 1 and is another double.
/// Above the bound, V8 writes the shortest digits padded with zeros
/// (`2**60` is `1152921504606847000`), so those take V8's conversion.
fn safe_integer(number: f64) -> Option<i64> {
    if number.abs() < SAFE_INTEGER_BOUND && number.fract() == 0.0 {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "an integer below 2^53 in magnitude converts exactly"
        )]
        Some(number as i64)
    } else {
        None
    }
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

/// Why [`push_utf16_json_string`] stopped.
#[derive(Clone, Copy)]
enum StringStop {
    /// A lone surrogate, this code unit.
    LoneSurrogate(u16),
    /// `out` passed its cap.
    TooLarge,
}

/// [`push_json_string`] for UTF-16 code units, failing on the first lone
/// surrogate with its code unit, or once `out` holds more than `max` bytes,
/// either leaving `out` partly written.
fn push_utf16_json_string(out: &mut String, units: &[u16], max: usize) -> Result<(), StringStop> {
    out.push('"');
    for c in char::decode_utf16(units.iter().copied()) {
        push_json_char(
            out,
            c.map_err(|error| StringStop::LoneSurrogate(error.unpaired_surrogate()))?,
        );
        if out.len() > max {
            return Err(StringStop::TooLarge);
        }
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
