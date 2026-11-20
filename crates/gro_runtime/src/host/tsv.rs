//! The native half of the tsv host module: tsv's formatters, its import
//! parse, and its ignore stacks, by tsv's own crates (`tsv_ts`,
//! `tsv_svelte`, `tsv_css`, `tsv_ignore`), each call's result what the
//! Node module, over tsv's N-API binding, returns for it.
//!
//! A member reads its arguments as the values they are, running no code of
//! a caller's: a source is a string, never a boxed one, read natively, and
//! one holding a lone surrogate is refused, as tsv's binding refuses it,
//! since UTF-8 would hold U+FFFD in its place. Options are `undefined`,
//! `null`, or an object that is no proxy, array, or function, read by its
//! own enumerable string keys, each one's descriptor read natively: a key
//! the function doesn't take is refused, and so is a `sourceType` that is a
//! getter, which isn't run. The refusals are `TypeError`s of tsv's own
//! texts, the names a message quotes escaped and cut as tsv's options
//! reader does ([`quote`]), but for the getter's, which is Gro's, and a
//! proxy's, refused as options that aren't an object. An ignore stack's paths and patterns are
//! strings, each lone surrogate read as U+FFFD, as the binding reads them.
//!
//! A source tsv can't format or parse is a result, `{ok: false, error}`,
//! never a throw: the error a plain object of its `kind`, `message`,
//! `frame`, and span, as tsv's binding builds it, each record and array a
//! new one of the realm's intrinsics, its properties defined, so no setter
//! of a patched prototype runs. A failure with no point in the source, a
//! source past tsv's size cap, throws an `Error` of its text. An import
//! record's offsets are UTF-16 ones of the language's coordinates: the
//! source's own for TypeScript, and, for Svelte, the source with a leading
//! byte-order mark removed ([`Utf16Offsets`]).
//!
//! Every call runs on the run's thread, sized by `STACK_SIZE` to tsv's
//! contract, where tsv's depth limits make input nested past them a parse
//! error, never a stack overflow. A panic in tsv, which only a bug of tsv's
//! is, ends the process: the release build aborts on it, and a debug one
//! can't unwind out of the native callback, so it aborts too. A stack's
//! precedence is push order, so a deeper layer pushed before a shallower one
//! loses to it, where git would have the deeper one win. A debug build of
//! `tsv_ignore` asserts layers are pushed shallowest-first, so there a push
//! above one already pushed (`'a/b'`, then `'a'`) ends the process, where
//! the release build and the Node module push it.
//!
//! An ignore stack is a handle of [`TSV_IGNORE_STACK`]'s kind (`handle.rs`),
//! made of the prototype the host made with the module's namespace, whose
//! state is a number naming its `tsv_ignore::IgnoreStack` in the run's
//! [`NativeTable`], in `OpState` ([`TsvState`]), so a stack lives as long as
//! its handle can be reached. It's counted to V8 by its size and its layers'
//! text as they're pushed, not the rules parsed from it, and a pop
//! subtracts nothing; nothing bounds a stack's memory but the process's.

use std::cell::OnceCell;
use std::fmt::Write as _;
use std::rc::Rc;

use deno_core::{OpState, v8};
use tsv_ignore::{IgnoreStack, PathKind};
use tsv_ts::{Error, Goal, Imports};

use super::handle::{self, NativeTable, TSV_IGNORE_STACK};
use super::{Thrown, native, own_data, string, throw_error, throw_type_error};

/// What a stack's method called on anything but a stack throws.
pub const NOT_A_STACK: &str =
    "an ignore stack's methods are called on an ignore stack, not another object";

/// What a push throws for an anchor or content that isn't a string.
pub const NOT_A_LAYER: &str = "an ignore layer's anchor and content are strings";

/// What `is_ignored` throws for a path that isn't a string or an `is_dir`
/// that isn't a boolean.
pub const NOT_A_PATH: &str = "is_ignored takes a path string and an is_dir boolean";

/// What the factory throws when the run holds no stack prototype, which
/// only a factory reached some other way than through the namespace meets.
const NO_PROTOTYPE: &str = "this run made no tsv namespace, so it holds no ignore stack prototype";

/// What a member throws in a runtime started without the module's state,
/// which only a runtime made outside a run's start is.
const NO_TSV_STATE: &str = "this runtime was started without tsv's state";

/// The longest string a refusal quotes whole, in UTF-16 units, as tsv's
/// options reader quotes one.
const QUOTED_MAX: usize = 40;

/// The tsv host module's state, put in every run's `OpState` when it
/// starts, empty: the table joins when a run makes its first stack.
#[derive(Default)]
pub struct TsvState {
    /// The ignore stacks.
    stacks: OnceCell<Rc<NativeTable<IgnoreStack>>>,
}

impl TsvState {
    /// How many stacks the run's table holds.
    #[cfg(test)]
    pub fn stacks_len(&self) -> usize {
        self.stacks.get().map_or(0, |stacks| stacks.len())
    }
}

/// The language a formatter or an import parse reads.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Language {
    /// TypeScript or JavaScript, the one language with a parse goal.
    TypeScript,
    /// A Svelte component.
    Svelte,
    /// CSS.
    Css,
}

/// The family of a member, which its refusals name.
#[derive(Clone, Copy)]
enum Noun {
    Format,
    Imports,
}

impl Noun {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Format => "format",
            Self::Imports => "imports",
        }
    }
}

/// `format_<language>(source, options)`: `{ok: true, code}`, the formatted
/// source, or `{ok: false, error}` for one tsv can't format. `None` with an
/// exception pending when it threw.
pub fn format<'s>(
    scope: &v8::PinScope<'s, '_>,
    language: Language,
    source: v8::Local<'s, v8::Value>,
    options: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    let text = read_source(scope, source, Noun::Format)?;
    let goal = read_options(scope, options, Noun::Format, language).ok()?;
    let formatted = match (language, goal) {
        (Language::TypeScript, Some(goal)) => tsv_ts::format_with_goal(&text, goal),
        (Language::TypeScript, None) => tsv_ts::format(&text),
        (Language::Svelte, _) => tsv_svelte::format(&text),
        (Language::Css, _) => tsv_css::format(&text),
    };
    match formatted {
        Ok(code) => {
            let ok = v8::Boolean::new(scope, true).into();
            let code = string(scope, &code)?.into();
            Some(record(scope, &[("ok", ok), ("code", code)])?.into())
        }
        Err(error) => failure(scope, &error),
    }
}

/// `parse_imports_typescript(source, options)` with `svelte` false, else
/// `parse_imports_svelte(source, options)`: `{ok: true, hasModuleSyntax,
/// imports}`, or `{ok: false, error}` for a source that doesn't parse.
/// `None` with an exception pending when it threw.
pub fn parse_imports<'s>(
    scope: &v8::PinScope<'s, '_>,
    svelte: bool,
    source: v8::Local<'s, v8::Value>,
    options: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    let text = read_source(scope, source, Noun::Imports)?;
    let language = if svelte {
        Language::Svelte
    } else {
        Language::TypeScript
    };
    let goal = read_options(scope, options, Noun::Imports, language).ok()?;
    let parsed = match (svelte, goal) {
        (true, _) => tsv_svelte::parse_imports(&text),
        (false, Some(goal)) => tsv_ts::parse_imports_with_goal(&text, goal),
        (false, None) => tsv_ts::parse_imports(&text),
    };
    match parsed {
        Ok(imports) => {
            let offsets = Utf16Offsets::new(&text, svelte);
            imports_result(scope, &imports, &text, &offsets)
        }
        Err(error) => failure(scope, &error),
    }
}

/// `{ok: true, hasModuleSyntax, imports}` of `imports`, collected from
/// `text`, each record's keys in the binding's order.
fn imports_result<'s>(
    scope: &v8::PinScope<'s, '_>,
    imports: &Imports,
    text: &str,
    offsets: &Utf16Offsets,
) -> Option<v8::Local<'s, v8::Value>> {
    let null: v8::Local<v8::Value> = v8::null(scope).into();
    let name_or_null = |name: Option<&str>| -> Option<v8::Local<'s, v8::Value>> {
        name.map_or(Some(null), |name| string(scope, name).map(Into::into))
    };
    let span = |start: u32, end: u32| -> [(&'static str, v8::Local<'s, v8::Value>); 2] {
        [
            ("start", number(scope, offsets.at(start))),
            ("end", number(scope, offsets.at(end))),
        ]
    };
    let mut records = Vec::with_capacity(imports.records.len());
    for record in &imports.records {
        let specifier = match record.specifier.zip(record.specifier_value(text)) {
            Some((at, value)) => {
                let [start, end] = span(at.start, at.end);
                let value = string(scope, &value)?.into();
                self::record(scope, &[("value", value), start, end])?.into()
            }
            None => null,
        };
        let attributes = match record.attributes {
            Some(at) => self::record(scope, &span(at.start, at.end))?.into(),
            None => null,
        };
        let [start, end] = span(record.span.start, record.span.end);
        let kind = string(scope, record.kind.as_str())?.into();
        let made = self::record(
            scope,
            &[
                ("kind", kind),
                start,
                end,
                ("specifier", specifier),
                ("typeOnly", name_or_null(record.type_only.as_str())?),
                ("phase", name_or_null(record.phase.as_str())?),
                ("attributes", attributes),
            ],
        )?;
        records.push(made.into());
    }
    let ok = v8::Boolean::new(scope, true).into();
    let has_module_syntax = v8::Boolean::new(scope, imports.has_module_syntax).into();
    let records = v8::Array::new_with_elements(scope, &records).into();
    let made = record(
        scope,
        &[
            ("ok", ok),
            ("hasModuleSyntax", has_module_syntax),
            ("imports", records),
        ],
    )?;
    Some(made.into())
}

/// `{ok: false, error}` of `error`, as tsv's binding returns a failure it
/// can point at in the source: `kind` `'refused'` for a format's refusal,
/// `'parse'` for anything else, its `message`, `frame`, `start`, `end`, and
/// `loc`. One with no point, a source past tsv's size cap, is thrown as an
/// `Error` of its text. `None` with an exception pending.
fn failure<'s>(scope: &v8::PinScope<'s, '_>, error: &Error) -> Option<v8::Local<'s, v8::Value>> {
    let Some(report) = error.to_report() else {
        throw_error(scope, &format!("{error:#}"));
        return None;
    };
    let kind = if report.kind == tsv_ts::ErrorKind::Refusal {
        "refused"
    } else {
        "parse"
    };
    let position = |point: tsv_ts::WirePoint| {
        let line = number(scope, point.line);
        let column = number(scope, point.column);
        record(scope, &[("line", line), ("column", column)]).map(Into::into)
    };
    let start = position(report.start)?;
    let end = position(report.end)?;
    let loc = record(scope, &[("start", start), ("end", end)])?.into();
    let kind = string(scope, kind)?.into();
    let message = string(scope, &report.message)?.into();
    let frame = string(scope, &report.frame)?.into();
    let error = record(
        scope,
        &[
            ("kind", kind),
            ("message", message),
            ("frame", frame),
            ("start", number(scope, report.start.offset)),
            ("end", number(scope, report.end.offset)),
            ("loc", loc),
        ],
    )?;
    let ok = v8::Boolean::new(scope, false).into();
    Some(record(scope, &[("ok", ok), ("error", error.into())])?.into())
}

/// A new object of the realm's `Object.prototype` holding `fields` as its
/// own data, in order, defined, so no setter of a patched prototype runs.
/// `None` with an exception pending when V8 failed to make it.
fn record<'s>(
    scope: &v8::PinScope<'s, '_>,
    fields: &[(&str, v8::Local<'s, v8::Value>)],
) -> Option<v8::Local<'s, v8::Object>> {
    let made = v8::Object::new(scope);
    for (key, value) in fields {
        let key = string(scope, key)?.into();
        made.create_data_property(scope, key, *value)?;
    }
    Some(made)
}

/// `value` as a JS number.
fn number<'s>(scope: &v8::PinScope<'s, '_>, value: u32) -> v8::Local<'s, v8::Value> {
    v8::Integer::new_from_unsigned(scope, value).into()
}

/// The UTF-16 offset of each byte offset of a source, in a language's
/// coordinates: a leading byte-order mark counted, as TypeScript's are, or
/// removed, as Svelte's are.
struct Utf16Offsets {
    /// The UTF-16 offset of each byte offset, `None` for an ASCII source,
    /// whose byte offsets are its UTF-16 ones.
    table: Option<Vec<u32>>,
}

impl Utf16Offsets {
    /// The offsets of `source`, its leading byte-order mark removed when
    /// `elide_bom`.
    fn new(source: &str, elide_bom: bool) -> Self {
        if source.is_ascii() {
            return Self { table: None };
        }
        let skip = if elide_bom && source.starts_with('\u{feff}') {
            '\u{feff}'.len_utf8()
        } else {
            0
        };
        let mut table = Vec::with_capacity(source.len() + 1);
        let mut at: u32 = 0;
        for (byte, ch) in source.char_indices() {
            // a byte inside a character reads as its start, never named by a
            // record, which tsv keeps on character boundaries
            table.resize(byte, at);
            table.push(at);
            if byte >= skip {
                at += u32::try_from(ch.len_utf16()).unwrap_or(2);
            }
        }
        table.resize(source.len(), at);
        table.push(at);
        Self { table: Some(table) }
    }

    /// The UTF-16 offset of `byte`.
    fn at(&self, byte: u32) -> u32 {
        self.table.as_ref().map_or(byte, |table| {
            usize::try_from(byte)
                .ok()
                .and_then(|byte| table.get(byte))
                .copied()
                .unwrap_or(byte)
        })
    }
}

/// `value` as the source of a member of `noun`'s family, or `None` with a
/// `TypeError` thrown when it isn't a string or holds a lone surrogate.
fn read_source(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    noun: Noun,
) -> Option<String> {
    let noun = noun.as_str();
    let Ok(source) = v8::Local::<v8::String>::try_from(value) else {
        let what = if value.is_null() {
            "null".to_owned()
        } else {
            value.type_of(scope).to_rust_string_lossy(scope)
        };
        throw_type_error(
            scope,
            &format!("{noun} source must be a string (got {what})"),
        );
        return None;
    };
    // a one-byte string holds no surrogate
    if source.is_onebyte() {
        return Some(source.to_rust_string_lossy(scope));
    }
    let units = utf16_of(scope, source);
    String::from_utf16(&units).map_or_else(
        |_| {
            let offset = lone_surrogate_offset(&units);
            throw_type_error(
                scope,
                &format!(
                    "{noun} source must be well-formed UTF-16 (a lone surrogate at offset {offset})"
                ),
            );
            None
        },
        Some,
    )
}

/// The UTF-16 code units of `string`.
fn utf16_of(scope: &v8::PinScope<'_, '_>, string: v8::Local<'_, v8::String>) -> Vec<u16> {
    let mut units = vec![0; string.length()];
    string.write_v2(scope, 0, &mut units, v8::WriteFlags::empty());
    units
}

/// The offset of the first lone surrogate in `units`, or their length when
/// they hold none.
fn lone_surrogate_offset(units: &[u16]) -> usize {
    let mut at = 0;
    while let Some(&unit) = units.get(at) {
        if is_high_surrogate(unit)
            && units
                .get(at + 1)
                .is_some_and(|&next| is_low_surrogate(next))
        {
            at += 2;
        } else if is_high_surrogate(unit) || is_low_surrogate(unit) {
            return at;
        } else {
            at += 1;
        }
    }
    at
}

const fn is_high_surrogate(unit: u16) -> bool {
    matches!(unit, 0xd800..=0xdbff)
}

const fn is_low_surrogate(unit: u16) -> bool {
    matches!(unit, 0xdc00..=0xdfff)
}

/// The goal `options` names for a member of `noun`'s family reading
/// `language`, `None` for none, or `Thrown` with a `TypeError` thrown for
/// options it refuses, or with what V8 threw pending.
fn read_options(
    scope: &v8::PinScope<'_, '_>,
    options: v8::Local<'_, v8::Value>,
    noun: Noun,
    language: Language,
) -> Result<Option<Goal>, Thrown> {
    let noun = noun.as_str();
    let typescript = language == Language::TypeScript;
    if options.is_null_or_undefined() {
        return Ok(None);
    }
    let object = v8::Local::<v8::Object>::try_from(options)
        .ok()
        .filter(|_| !options.is_function() && !options.is_array() && !options.is_proxy());
    let Some(object) = object else {
        throw_type_error(scope, &format!("{noun} options must be an object"));
        return Err(Thrown);
    };
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
        .ok_or(Thrown)?;
    let mut goal = None;
    for index in 0..keys.length() {
        let key = keys.get_index(scope, index).ok_or(Thrown)?;
        let key = v8::Local::<v8::String>::try_from(key).map_err(|_| Thrown)?;
        let units = utf16_of(scope, key);
        if units != "sourceType".encode_utf16().collect::<Vec<_>>() {
            let expected = if typescript {
                "expected 'sourceType'"
            } else {
                "this export takes no options"
            };
            let quoted = quote(&units);
            throw_type_error(
                scope,
                &format!("unknown {noun} option {quoted} ({expected})"),
            );
            return Err(Thrown);
        }
        let Some(value) = own_data(scope, object, key.into())? else {
            throw_type_error(
                scope,
                &format!("{noun} option 'sourceType' must be data, not a getter"),
            );
            return Err(Thrown);
        };
        if value.is_undefined() {
            continue;
        }
        if !typescript {
            throw_type_error(
                scope,
                &format!("{noun} option 'sourceType' is only supported for TypeScript"),
            );
            return Err(Thrown);
        }
        let named = v8::Local::<v8::String>::try_from(value)
            .ok()
            .map(|named| named.to_rust_string_lossy(scope));
        goal = Some(match named.as_deref() {
            Some("script") => Goal::Script,
            Some("module") => Goal::Module,
            _ => {
                let got = describe(scope, value);
                throw_type_error(
                    scope,
                    &format!("{noun} option 'sourceType' must be 'script' or 'module' (got {got})"),
                );
                return Err(Thrown);
            }
        });
    }
    Ok(goal)
}

/// A refused value as a refusal names it, as tsv's options reader does: a
/// string quoted ([`quote`]), a number or boolean as JS writes it, `null`,
/// and anything else by its `typeof`. Converts nothing a caller can answer:
/// a primitive number or boolean is written by V8 alone.
fn describe(scope: &v8::PinScope<'_, '_>, value: v8::Local<'_, v8::Value>) -> String {
    if let Ok(text) = v8::Local::<v8::String>::try_from(value) {
        return quote(&utf16_of(scope, text));
    }
    if value.is_number() || value.is_boolean() {
        return value
            .to_string(scope)
            .map_or_else(String::new, |text| text.to_rust_string_lossy(scope));
    }
    if value.is_null() {
        return "null".to_owned();
    }
    value.type_of(scope).to_rust_string_lossy(scope)
}

/// `units` single-quoted, as tsv's options reader quotes a name it refuses:
/// escaped as `JSON.stringify` escapes a string, but `"` left as it is and
/// `'` escaped, each character JSON leaves raw that a terminal reads as a
/// line break, a control, or a reordering written `\uXXXX`, and cut past
/// [`QUOTED_MAX`] units, one sooner where the cut would split a surrogate
/// pair, with a `…` inside the quotes.
fn quote(units: &[u16]) -> String {
    let mut cut = units.len();
    if cut > QUOTED_MAX {
        cut = QUOTED_MAX;
        if units
            .get(cut - 1)
            .is_some_and(|&last| is_high_surrogate(last))
        {
            cut -= 1;
        }
    }
    let kept = units.get(..cut).unwrap_or(units);
    let mut quoted = String::from("'");
    let mut at = 0;
    while let Some(&unit) = kept.get(at) {
        if is_high_surrogate(unit) && kept.get(at + 1).is_some_and(|&next| is_low_surrogate(next)) {
            quoted.push_str(&String::from_utf16_lossy(&kept[at..at + 2]));
            at += 2;
            continue;
        }
        at += 1;
        match unit {
            0x5c => quoted.push_str("\\\\"),
            0x27 => quoted.push_str("\\'"),
            0x08 => quoted.push_str("\\b"),
            0x0c => quoted.push_str("\\f"),
            0x0a => quoted.push_str("\\n"),
            0x0d => quoted.push_str("\\r"),
            0x09 => quoted.push_str("\\t"),
            0x00..=0x1f
            | 0x7f..=0x9f
            | 0x061c
            | 0x200e
            | 0x200f
            | 0x2028
            | 0x2029
            | 0x202a..=0x202e
            | 0x2066..=0x2069
            | 0xd800..=0xdfff => {
                let _ = write!(quoted, "\\u{unit:04x}");
            }
            _ => quoted.push(char::from_u32(u32::from(unit)).unwrap_or('\u{fffd}')),
        }
    }
    if cut < units.len() {
        quoted.push('\u{2026}');
    }
    quoted.push('\'');
    quoted
}

/// The layer a push adds.
#[derive(Clone, Copy)]
pub enum Layer {
    /// A `.gitignore`'s.
    Gitignore,
    /// A `.formatignore`'s, an override.
    Formatignore,
    /// A `.prettierignore`'s, an override matched as a `.formatignore`'s.
    Prettierignore,
}

/// The layers a pop takes from.
#[derive(Clone, Copy)]
pub enum Layers {
    /// The `.gitignore` layers.
    Gitignore,
    /// The override layers.
    Override,
}

/// `create_ignore_stack()`: a new stack, made of the prototype the run
/// made with the namespace, ignoring nothing. `None` with an exception
/// pending when a step failed.
pub fn create_ignore_stack<'s>(
    state: &OpState,
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Value>> {
    let Some(tsv) = state.try_borrow::<TsvState>() else {
        throw_error(scope, NO_TSV_STATE);
        return None;
    };
    let Some(prototype) = native::prototype(state, scope, &TSV_IGNORE_STACK) else {
        throw_error(scope, NO_PROTOTYPE);
        return None;
    };
    let made = tsv.stacks.get_or_init(Rc::default).make(
        scope,
        &TSV_IGNORE_STACK,
        prototype,
        IgnoreStack::new(),
    )?;
    Some(made.into())
}

/// `stack.push_<layer>(anchor, content)` called on `receiver`: the layer
/// pushed, or a `TypeError` thrown when the receiver is no stack or the
/// anchor or content isn't a string, the receiver checked first.
pub fn push(
    state: &OpState,
    scope: &mut v8::PinScope<'_, '_>,
    receiver: v8::Local<'_, v8::Value>,
    layer: Layer,
    anchor: v8::Local<'_, v8::Value>,
    content: v8::Local<'_, v8::Value>,
) {
    let Some(id) = read(scope, receiver) else {
        return;
    };
    let (Ok(anchor), Ok(content)) = (
        v8::Local::<v8::String>::try_from(anchor),
        v8::Local::<v8::String>::try_from(content),
    ) else {
        throw_type_error(scope, NOT_A_LAYER);
        return;
    };
    let anchor = anchor.to_rust_string_lossy(scope);
    let content = content.to_rust_string_lossy(scope);
    let Some(stacks) = stacks(state) else {
        throw_error(scope, NO_TSV_STATE);
        return;
    };
    let pushed = stacks.with(id, |stack| match layer {
        Layer::Gitignore => stack.push_gitignore(&anchor, &content),
        Layer::Formatignore => stack.push_formatignore(&anchor, &content),
        Layer::Prettierignore => stack.push_prettierignore(&anchor, &content),
    });
    if pushed.is_none() {
        throw_error(scope, NO_TSV_STATE);
        return;
    }
    stacks.grow(scope, id, anchor.len() + content.len());
}

/// `stack.pop_gitignore()` or `stack.pop_override()` called on `receiver`:
/// the last layer of `layers` popped, if any, or a `TypeError` thrown when
/// the receiver is no stack.
pub fn pop(
    state: &OpState,
    scope: &v8::PinScope<'_, '_>,
    receiver: v8::Local<'_, v8::Value>,
    layers: Layers,
) {
    let Some(id) = read(scope, receiver) else {
        return;
    };
    let popped = stacks(state).and_then(|stacks| {
        stacks.with(id, |stack| match layers {
            Layers::Gitignore => stack.pop_gitignore(),
            Layers::Override => stack.pop_override(),
        })
    });
    if popped.is_none() {
        throw_error(scope, NO_TSV_STATE);
    }
}

/// `stack.is_ignored(path, is_dir)` called on `receiver`: whether `path`
/// is ignored, or a `TypeError` thrown when the receiver is no stack, the
/// path isn't a string, or `is_dir` isn't a boolean, the receiver checked
/// first.
pub fn is_ignored<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
    path: v8::Local<'s, v8::Value>,
    is_dir: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    let id = read(scope, receiver)?;
    let Ok(path) = v8::Local::<v8::String>::try_from(path) else {
        throw_type_error(scope, NOT_A_PATH);
        return None;
    };
    if !is_dir.is_boolean() {
        throw_type_error(scope, NOT_A_PATH);
        return None;
    }
    let kind = PathKind::from_is_dir(is_dir.boolean_value(scope));
    let path = path.to_rust_string_lossy(scope);
    let Some(ignored) =
        stacks(state).and_then(|stacks| stacks.with(id, |stack| stack.is_ignored(&path, kind)))
    else {
        throw_error(scope, NO_TSV_STATE);
        return None;
    };
    Some(v8::Boolean::new(scope, ignored).into())
}

/// The run's table of stacks, `None` when the runtime holds no tsv state
/// or has made no stack.
fn stacks(state: &OpState) -> Option<Rc<NativeTable<IgnoreStack>>> {
    state.try_borrow::<TsvState>()?.stacks.get().cloned()
}

/// The number the stack `receiver` is holds, or `None` with a `TypeError`
/// thrown when it's no stack.
fn read(scope: &v8::PinScope<'_, '_>, receiver: v8::Local<'_, v8::Value>) -> Option<u64> {
    let id = handle::read_number(scope, &TSV_IGNORE_STACK, receiver);
    if id.is_none() {
        throw_type_error(scope, NOT_A_STACK);
    }
    id
}

#[cfg(test)]
mod tests {
    use super::{Utf16Offsets, lone_surrogate_offset, quote};

    fn units(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    #[test]
    fn a_quoted_name_is_escaped_as_tsv_quotes_it() {
        assert_eq!(quote(&units("locatons")), "'locatons'");
        assert_eq!(quote(&units("a\"b'c\\d")), "'a\"b\\'c\\\\d'");
        assert_eq!(
            quote(&units("\n\t\u{1}\u{7f}\u{2028}")),
            "'\\n\\t\\u0001\\u007f\\u2028'"
        );
        assert_eq!(quote(&[0x61, 0xd800, 0x62]), "'a\\ud800b'");
        assert_eq!(quote(&units("\u{1f600}")), "'\u{1f600}'");
        let long = "x".repeat(41);
        assert_eq!(
            quote(&units(&long)),
            format!("'{}\u{2026}'", "x".repeat(40))
        );
        // a cut that would split a pair is one sooner
        let split = format!("{}\u{1f600}", "x".repeat(39));
        assert_eq!(
            quote(&units(&split)),
            format!("'{}\u{2026}'", "x".repeat(39))
        );
    }

    #[test]
    fn a_lone_surrogate_is_found_where_it_is() {
        assert_eq!(lone_surrogate_offset(&[0x61, 0xd83d, 0xde00, 0xdc00]), 3);
        assert_eq!(lone_surrogate_offset(&[0xd800]), 0);
        assert_eq!(lone_surrogate_offset(&[0x61, 0x62]), 2);
    }

    #[test]
    fn byte_offsets_become_utf16_ones() {
        let source = "\u{feff}a\u{e9}\u{1f600}b";
        let counted = Utf16Offsets::new(source, false);
        let elided = Utf16Offsets::new(source, true);
        // the BOM, `a`, `é` (two bytes), the emoji (four bytes, two units), `b`
        let starts = [0, 3, 4, 6, 10, 11];
        assert_eq!(starts.map(|byte| counted.at(byte)), [0, 1, 2, 3, 5, 6]);
        assert_eq!(starts.map(|byte| elided.at(byte)), [0, 0, 1, 2, 4, 5]);
        assert_eq!(Utf16Offsets::new("abc", true).at(2), 2);
    }
}
