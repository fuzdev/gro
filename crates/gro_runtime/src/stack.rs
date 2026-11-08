//! An error's stack: what V8 captures when an error is made, and how its
//! `.stack` is formatted.
//!
//! [`install`] runs on every runtime, once `deno_core` has built its isolate.
//! It does two things:
//!
//! - It turns off V8's capture of a detailed stack trace for uncaught
//!   exceptions, which `deno_core` turns on. With it on, V8 walks the stack
//!   again on every error made, thrown or not, and keeps a second trace beside
//!   the one `.stack` is formatted from, whatever `Error.stackTraceLimit` says.
//!   Only an inspector reads that trace (and V8 turns the capture back on
//!   itself when a front-end enables its runtime domain); gro's reports read
//!   `.stack`, and a `JsError`'s frames come from the call sites `.stack` is
//!   formatted from, so turning it off loses nothing gro shows. It's a setting
//!   of the isolate, never stored in a snapshot.
//! - It replaces `deno_core`'s prepare-stack-trace callback with
//!   [`prepare_stack_trace`], which formats the same text with far fewer calls
//!   between Rust and JS per frame: it calls each call site's getters
//!   directly, looking each up once per error, reads only the ones a frame's
//!   text needs, and parses a file name as a URL only when it isn't a plain
//!   `file:` or `ext:` one. It applies no source map. It hands the error to
//!   `deno_core`'s callback when the program sets `Error.prepareStackTrace`.
//!
//! fgro applies no source maps, as Node doesn't by default: a position names
//! the code that ran. TypeScript needs none, since the type strip leaves every
//! line and UTF-16 column where it was; bundled JS shows positions in the
//! bundle. The vendored `deno_core` is patched so its `SourceMapper` keeps no
//! module's source map URL and maps no location, which covers every path that
//! reads one: a `JsError`'s frames, the call sites a program's
//! `Error.prepareStackTrace` reads, and a failed import's position.
//!
//! The text is `deno_core`'s, character for character: the error's `name:
//! message`, then a line `    at ...` per call site, a file URL printed
//! percent-decoded, as `deno_core` prints it. The tests here check it against
//! `deno_core`'s callback on the same errors.

use std::fmt::Write as _;

use deno_core::error::{call_site_evals_key, format_file_name, prepare_stack_trace_callback};
use deno_core::v8;

/// Turns off the isolate's capture of detailed stack traces for uncaught
/// exceptions and puts [`prepare_stack_trace`] in place of `deno_core`'s
/// callback.
pub fn install(isolate: &mut v8::Isolate) {
    // `(true, 0)` still captures on every error; only off skips it
    isolate.set_capture_stack_trace_for_uncaught_exceptions(false, 0);
    isolate.set_prepare_stack_trace_callback(prepare_stack_trace);
}

/// Formats `error`'s stack from `callsites` as `deno_core`'s callback does,
/// keeping the call sites on the error where `deno_core` reads them, and
/// handing the error to that callback when the program sets
/// `Error.prepareStackTrace`.
fn prepare_stack_trace<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    error: v8::Local<'s, v8::Value>,
    callsites: v8::Local<'s, v8::Array>,
) -> v8::Local<'s, v8::Value> {
    // `deno_core`'s callback stores the call sites itself
    if has_user_prepare_stack_trace(scope) {
        return prepare_stack_trace_callback(scope, error, callsites);
    }
    // where `deno_core`'s `JsError` reads its frames from, and its dynamic
    // import handling checks for call sites
    if let Ok(object) = error.try_cast::<v8::Object>() {
        let key = call_site_evals_key(scope);
        object.set_private(scope, key, callsites.into());
    }
    let mut text = String::new();
    push_header(scope, &mut text, error);
    // a getter's exception is caught here and dropped when the scope ends,
    // never left pending, as `deno_core` catches one per frame
    v8::tc_scope!(let scope, scope);
    let mut sites = Sites::default();
    for index in 0..callsites.length() {
        let Some(callsite) = callsites
            .get_index(scope, index)
            .and_then(|value| value.try_cast::<v8::Object>().ok())
        else {
            break;
        };
        let start = text.len();
        text.push_str("\n    at ");
        // `deno_core` stops at a call site it can't read (and warns, which
        // gro leaves to the stack it has)
        if push_frame(scope, &mut sites, callsite, &mut text).is_err() {
            text.truncate(start);
            break;
        }
    }
    v8::String::new(scope, &text).map_or_else(|| v8::undefined(scope).into(), Into::into)
}

/// Whether `Error.prepareStackTrace` is a function, read as `deno_core`'s
/// callback reads it, which then calls it (reading it again, so a getter
/// there runs twice).
fn has_user_prepare_stack_trace(scope: &v8::PinScope<'_, '_>) -> bool {
    let global = scope.get_current_context().global(scope);
    let (Some(error_key), Some(prepare_key)) =
        (intern(scope, b"Error"), intern(scope, b"prepareStackTrace"))
    else {
        return false;
    };
    global
        .get(scope, error_key.into())
        .and_then(|value| value.try_cast::<v8::Object>().ok())
        .and_then(|error| error.get(scope, prepare_key.into()))
        .is_some_and(|value| value.is_function())
}

/// Appends the stack's first line: `name: message`, `name` or `message` alone
/// when the other is empty, and `Error` for a missing `name`, read in
/// `deno_core`'s order, `message` first.
fn push_header(scope: &v8::PinScope<'_, '_>, text: &mut String, error: v8::Local<'_, v8::Value>) {
    let Ok(object) = error.try_cast::<v8::Object>() else {
        return;
    };
    let read = |key: &[u8]| {
        let key = intern(scope, key)?;
        let value = object.get(scope, key.into())?;
        if value.is_undefined() {
            return None;
        }
        Some(value.to_string(scope)?.to_rust_string_lossy(scope))
    };
    let message = read(b"message").unwrap_or_default();
    let name = read(b"name").unwrap_or_else(|| "Error".to_owned());
    match (message.is_empty(), name.is_empty()) {
        (false, false) => {
            text.push_str(&name);
            text.push_str(": ");
            text.push_str(&message);
        }
        (false, true) => text.push_str(&message),
        (true, false) => text.push_str(&name),
        (true, true) => {}
    }
}

/// A frame that wasn't formatted: a getter threw or returned what V8's never
/// do.
struct Failed;

/// The call site getters a frame reads.
#[derive(Clone, Copy)]
enum Getter {
    FileName,
    LineNumber,
    ColumnNumber,
    EvalOrigin,
    TypeName,
    FunctionName,
    MethodName,
    IsToplevel,
    IsEval,
    IsNative,
    IsConstructor,
    IsAsync,
    IsPromiseAll,
    PromiseIndex,
}

impl Getter {
    /// How many getters there are, the size of [`Sites`]' table.
    const COUNT: usize = Self::PromiseIndex as usize + 1;

    /// The getter's property name on a call site.
    const fn name(self) -> &'static [u8] {
        match self {
            Self::FileName => b"getFileName",
            Self::LineNumber => b"getLineNumber",
            Self::ColumnNumber => b"getColumnNumber",
            Self::EvalOrigin => b"getEvalOrigin",
            Self::TypeName => b"getTypeName",
            Self::FunctionName => b"getFunctionName",
            Self::MethodName => b"getMethodName",
            Self::IsToplevel => b"isToplevel",
            Self::IsEval => b"isEval",
            Self::IsNative => b"isNative",
            Self::IsConstructor => b"isConstructor",
            Self::IsAsync => b"isAsync",
            Self::IsPromiseAll => b"isPromiseAll",
            Self::PromiseIndex => b"getPromiseIndex",
        }
    }
}

/// The getters of one error's call sites, each looked up on first use: V8
/// makes every call site from one template, so a getter found on one is the
/// getter of all.
#[derive(Default)]
struct Sites<'s> {
    getters: [Option<v8::Local<'s, v8::Function>>; Getter::COUNT],
}

impl<'s> Sites<'s> {
    /// Calls `getter` on `callsite`.
    fn call(
        &mut self,
        scope: &v8::PinScope<'s, '_>,
        callsite: v8::Local<'s, v8::Object>,
        getter: Getter,
    ) -> Result<v8::Local<'s, v8::Value>, Failed> {
        let slot = &mut self.getters[getter as usize];
        let function = if let Some(function) = slot {
            *function
        } else {
            let key = intern(scope, getter.name()).ok_or(Failed)?;
            let function = callsite
                .get(scope, key.into())
                .and_then(|value| value.try_cast::<v8::Function>().ok())
                .ok_or(Failed)?;
            *slot = Some(function);
            function
        };
        function.call(scope, callsite.into(), &[]).ok_or(Failed)
    }

    /// A getter's string, `None` for `null` or `undefined`.
    fn string(
        &mut self,
        scope: &v8::PinScope<'s, '_>,
        callsite: v8::Local<'s, v8::Object>,
        getter: Getter,
    ) -> Result<Option<String>, Failed> {
        let value = self.call(scope, callsite, getter)?;
        if value.is_null_or_undefined() {
            return Ok(None);
        }
        let string = value.try_cast::<v8::String>().map_err(|_| Failed)?;
        Ok(Some(string.to_rust_string_lossy(scope)))
    }

    /// A getter's integer, `None` for `null` or `undefined`.
    fn integer(
        &mut self,
        scope: &v8::PinScope<'s, '_>,
        callsite: v8::Local<'s, v8::Object>,
        getter: Getter,
    ) -> Result<Option<i64>, Failed> {
        let value = self.call(scope, callsite, getter)?;
        if value.is_null_or_undefined() {
            return Ok(None);
        }
        if !value.is_number() {
            return Err(Failed);
        }
        value.integer_value(scope).map(Some).ok_or(Failed)
    }

    /// A getter's boolean, `None` for `null` or `undefined`.
    fn flag(
        &mut self,
        scope: &v8::PinScope<'s, '_>,
        callsite: v8::Local<'s, v8::Object>,
        getter: Getter,
    ) -> Result<Option<bool>, Failed> {
        let value = self.call(scope, callsite, getter)?;
        if value.is_null_or_undefined() {
            return Ok(None);
        }
        if !value.is_boolean() {
            return Err(Failed);
        }
        Ok(Some(value.is_true()))
    }
}

/// Appends a frame, after its `at `, as `deno_core`'s `format_frame` does,
/// reading only the getters its text needs.
fn push_frame<'s>(
    scope: &v8::PinScope<'s, '_>,
    sites: &mut Sites<'s>,
    callsite: v8::Local<'s, v8::Object>,
    text: &mut String,
) -> Result<(), Failed> {
    if sites.flag(scope, callsite, Getter::IsAsync)? == Some(true) {
        text.push_str("async ");
    }
    if sites.flag(scope, callsite, Getter::IsPromiseAll)? == Some(true) {
        let index = sites
            .integer(scope, callsite, Getter::PromiseIndex)?
            .unwrap_or_default();
        write!(text, "Promise.all (index {index})").unwrap_or_default();
        return Ok(());
    }
    let is_top_level = sites.flag(scope, callsite, Getter::IsToplevel)? == Some(true);
    let is_constructor = sites.flag(scope, callsite, Getter::IsConstructor)? == Some(true);
    let function_name = sites.string(scope, callsite, Getter::FunctionName)?;
    if !(is_top_level || is_constructor) {
        let type_name = sites.string(scope, callsite, Getter::TypeName)?;
        let method_name = sites.string(scope, callsite, Getter::MethodName)?;
        if let Some(function_name) = &function_name {
            if let Some(type_name) = &type_name
                && !function_name.starts_with(type_name.as_str())
            {
                text.push_str(type_name);
                text.push('.');
            }
            text.push_str(function_name);
            if let Some(method_name) = &method_name
                && !function_name.ends_with(method_name.as_str())
            {
                text.push_str(" [as ");
                text.push_str(method_name);
                text.push(']');
            }
        } else {
            if let Some(type_name) = &type_name {
                text.push_str(type_name);
                text.push('.');
            }
            text.push_str(method_name.as_deref().unwrap_or("<anonymous>"));
        }
    } else if is_constructor {
        text.push_str("new ");
        text.push_str(function_name.as_deref().unwrap_or("<anonymous>"));
    } else if let Some(function_name) = &function_name {
        text.push_str(function_name);
    } else {
        return push_location(scope, sites, callsite, text);
    }
    text.push_str(" (");
    push_location(scope, sites, callsite, text)?;
    text.push(')');
    Ok(())
}

/// Appends a frame's location as `deno_core`'s `format_location` does, with
/// no working directory to shorten paths against.
fn push_location<'s>(
    scope: &v8::PinScope<'s, '_>,
    sites: &mut Sites<'s>,
    callsite: v8::Local<'s, v8::Object>,
    text: &mut String,
) -> Result<(), Failed> {
    let file_name = sites.string(scope, callsite, Getter::FileName)?;
    if sites.flag(scope, callsite, Getter::IsNative)? == Some(true) {
        text.push_str("native");
        return Ok(());
    }
    let file_name = file_name.unwrap_or_default();
    let is_wasm = file_name.starts_with("wasm://");
    if file_name.is_empty() {
        if sites.flag(scope, callsite, Getter::IsEval)? == Some(true)
            && let Some(eval_origin) = sites.string(scope, callsite, Getter::EvalOrigin)?
        {
            push_eval_origin(text, &eval_origin);
            text.push_str(", ");
        }
        text.push_str("<anonymous>");
    } else {
        push_file_name(text, &file_name);
    }
    let line_number = sites.integer(scope, callsite, Getter::LineNumber)?;
    let column_number = sites.integer(scope, callsite, Getter::ColumnNumber)?;
    if is_wasm {
        // `url:wasm-function[index]:0xoffset`, from V8's 1-based function
        // index and byte offset
        if let Some(line_number) = line_number {
            write!(text, ":wasm-function[{}]", line_number - 1).unwrap_or_default();
        }
        if let Some(column_number) = column_number {
            write!(text, ":0x{:x}", column_number - 1).unwrap_or_default();
        }
    } else if let Some(line_number) = line_number {
        write!(text, ":{line_number}").unwrap_or_default();
        if let Some(column_number) = column_number {
            write!(text, ":{column_number}").unwrap_or_default();
        }
    }
    Ok(())
}

/// Appends `file_name` as `deno_core`'s `format_file_name` prints it with no
/// working directory: a `file:` or `ext:` URL with no `%` in it prints as is,
/// and the rest go through `deno_core`'s function, which percent-decodes them
/// and shortens a long `data:` URL.
fn push_file_name(text: &mut String, file_name: &str) {
    if !file_name.contains('%')
        && (file_name.starts_with("file:///") || file_name.starts_with("ext:"))
    {
        text.push_str(file_name);
    } else {
        let parts = format_file_name(file_name, None);
        if let Some(working_dir_path) = &parts.working_dir_path {
            text.push_str(working_dir_path);
            text.push_str(parts.file_name.trim_start_matches("./"));
        } else {
            text.push_str(&parts.file_name);
        }
    }
}

/// Appends an eval origin as `deno_core`'s `format_eval_origin` does: its
/// innermost location's file name formatted as a frame's is, the rest as V8
/// wrote it.
fn push_eval_origin(text: &mut String, eval_origin: &str) {
    let Some((before, (file, line, column), after)) = parse_eval_origin(eval_origin) else {
        text.push_str(eval_origin);
        return;
    };
    text.push_str(before);
    push_file_name(text, file);
    write!(text, ":{line}:{column}{after}").unwrap_or_default();
}

/// Splits an eval origin, `eval at name (origin)` with `origin` a location or
/// another eval origin, at its innermost location, as `deno_core`'s
/// `parse_eval_origin` does: the text before it, its file name, line, and
/// column, and the text after it.
fn parse_eval_origin(eval_origin: &str) -> Option<(&str, (&str, i64, i64), &str)> {
    const EVAL_AT: &str = "eval at ";
    // only the innermost eval origin has a location
    let mut innermost_start = eval_origin.rfind(EVAL_AT)? + EVAL_AT.len();
    // past the function name
    innermost_start += eval_origin[innermost_start..].find('(')? + 1;
    if innermost_start >= eval_origin.len() {
        return None;
    }
    // from the right: the column (with what follows it), the line, the file
    let mut parts = eval_origin[innermost_start..].rsplitn(3, ':');
    let column_with_rest = parts.next()?;
    let column_end = column_with_rest.find(')')?;
    let column = column_with_rest[..column_end].parse::<i64>().ok()?;
    let line = parts.next()?.parse::<i64>().ok()?;
    let file = parts.next()?;
    let column_start = eval_origin.rfind(':')? + 1;
    let innermost_end = column_start + column_end;
    Some((
        &eval_origin[..innermost_start],
        (file, line, column),
        &eval_origin[innermost_end..],
    ))
}

/// `name` as an internalized V8 string, so property lookups by it skip
/// interning it again.
fn intern<'s>(scope: &v8::PinScope<'s, '_>, name: &[u8]) -> Option<v8::Local<'s, v8::String>> {
    v8::String::new_from_one_byte(scope, name, v8::NewStringType::Internalized)
}

#[cfg(test)]
mod tests;
