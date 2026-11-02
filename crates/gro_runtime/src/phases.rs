//! Phase timings: a timestamp at each phase boundary of a run, for the
//! measurement harness, recorded only under the `phases` cargo feature.
//!
//! Without the feature, [`start`], [`mark`], [`open`], and [`finish`] are
//! empty and inlined away, so a default build carries no clock read, no
//! static, and no environment read for them. Release and snapshot builds
//! never enable it; `cargo xtask bench --phases` builds a binary that does.
//!
//! With it, [`start`] reads the wall clock (`CLOCK_REALTIME`, as nanoseconds
//! since the Unix epoch) and the monotonic clock once, back to back, at the
//! embedder's entry; each [`mark`] records the monotonic time since then; and
//! [`finish`] marks [`Phase::Exit`] and writes the marks to the file the
//! environment variable [`PHASES_FILE_ENV`] names, when it's set. The wall
//! anchor is what lets the harness, reading the same clock, price the spans
//! outside the process's own: from its spawn to the entry, and from
//! [`Phase::Exit`] to the exit being reaped. A confined process opens the file
//! with [`open`] before its box goes up, since the box refuses new opens, and
//! writes through that handle; nothing goes to stdout or stderr.
//!
//! The file is text, one record per line, fields separated by a space:
//!
//! ```text
//! gro_phases 1
//! main_unix_ns 1761998400123456789
//! engine 1203456
//! runtime 2310987
//! exit 6012345
//! ```
//!
//! The first line names the format and its version; `main_unix_ns` is the
//! wall anchor; each later line is a phase's name and the nanoseconds from
//! the entry to the phase's end, in the order the marks were taken. A phase
//! marked twice keeps both lines.

/// Whether this build records phases: the `phases` feature of this crate,
/// which is what decides it, so an embedder names a phases build by this
/// rather than by a feature of its own.
pub const ENABLED: bool = cfg!(feature = "phases");

/// The variable naming the file [`finish`] writes the marks to.
pub const PHASES_FILE_ENV: &str = "GRO_PHASES_FILE";

/// The first line of the file, naming its format and version.
pub const PHASES_HEADER: &str = "gro_phases 1";

/// A phase boundary, named by the phase it ends.
///
/// Each phase's span runs from the mark before it (or the entry) to its
/// own, so it holds everything in between, not only its namesake; each
/// variant's doc says what that is in `fgro`. A failed run stops marking
/// where it failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// `fgro eval`, from the entry: parsing the arguments, closing the
    /// inherited descriptors, creating the phases file, reading the working
    /// directory and the stdin driver, then the pre-pass on a thread of its
    /// own (the entry resolved, the box's roots found and opened) and the
    /// join.
    Prepass,
    /// `fgro eval`'s snapshot check and its box installed: Landlock and
    /// seccomp.
    Confine,
    /// V8's process-wide setup (`engine::init_engine`). In `fgro run`, from
    /// the entry, so also parsing the arguments and starting the sized
    /// thread; in eval, also starting the thread that evaluates.
    Engine,
    /// The runtime made: in `fgro run`, the entry resolved and the loader
    /// set up; in eval, the engine and options checked, the out-of-memory
    /// backstop set, and the loader set up; then, in both, the tokio
    /// runtime, and the isolate and extensions from the snapshot when there
    /// is one.
    Runtime,
    /// The bootstrap's per-run steps, which in eval install
    /// `determinism: deny`.
    Bootstrap,
    /// Eval's guards: the heap watch, the timeout's watchdog thread, and the
    /// time zone check.
    Guards,
    /// Eval's driver loaded, evaluated, and its exports read.
    Driver,
    /// The entry's module graph loaded: resolved, read, stripped, compiled,
    /// and instantiated.
    Load,
    /// The entry evaluated: in `fgro run`, its evaluation and the event loop
    /// drained; in eval, its evaluation settled and the value made, by the
    /// driver's call or the export read.
    Evaluate,
    /// Eval's value serialized.
    Serialize,
    /// Eval's event loop drained and the driver's `after_drain` called.
    Drain,
    /// The runtime and the tokio runtime dropped.
    Teardown,
    /// Eval's envelope finished: the graph's access, its dump, the box
    /// checks, and the cap.
    Envelope,
    /// Eval's evaluating thread joined and the envelope written to stdout.
    Write,
    /// The embedder about to return from its entry: in `fgro run`, the
    /// result mapped and the sized thread joined.
    Exit,
}

impl Phase {
    /// The name the file writes.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Prepass => "prepass",
            Self::Confine => "confine",
            Self::Engine => "engine",
            Self::Runtime => "runtime",
            Self::Bootstrap => "bootstrap",
            Self::Guards => "guards",
            Self::Driver => "driver",
            Self::Load => "load",
            Self::Evaluate => "evaluate",
            Self::Serialize => "serialize",
            Self::Drain => "drain",
            Self::Teardown => "teardown",
            Self::Envelope => "envelope",
            Self::Write => "write",
            Self::Exit => "exit",
        }
    }
}

/// The file's text: the header, `wall` (the entry's wall clock, in
/// nanoseconds since the Unix epoch), then each mark, its nanoseconds since
/// the entry.
pub fn render(wall: u128, marks: &[(Phase, u128)]) -> String {
    let mut text = format!("{PHASES_HEADER}\nmain_unix_ns {wall}\n");
    for (phase, since) in marks {
        text.push_str(phase.name());
        text.push(' ');
        text.push_str(&since.to_string());
        text.push('\n');
    }
    text
}

#[cfg(feature = "phases")]
mod enabled {
    use std::fs::File;
    use std::io::Write as _;
    use std::sync::{Mutex, MutexGuard};
    use std::time::{Instant, SystemTime};

    use super::{PHASES_FILE_ENV, Phase, render};

    /// Where the file goes: not yet decided, nowhere, or this handle.
    #[derive(Debug)]
    enum Output {
        Unopened,
        None,
        File(File),
    }

    #[derive(Debug)]
    struct State {
        /// The monotonic and wall times read at [`start`].
        origin: Option<(Instant, u128)>,
        marks: Vec<(Phase, u128)>,
        output: Output,
    }

    static STATE: Mutex<State> = Mutex::new(State {
        origin: None,
        marks: Vec::new(),
        output: Output::Unopened,
    });

    fn state() -> MutexGuard<'static, State> {
        // a panic while holding it leaves the marks usable
        STATE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn start() {
        let wall = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        let now = Instant::now();
        let mut state = state();
        state.origin.get_or_insert((now, wall));
    }

    pub fn mark(phase: Phase) {
        let now = Instant::now();
        let mut state = state();
        if let Some((origin, _)) = state.origin {
            let since = now.duration_since(origin).as_nanos();
            state.marks.push((phase, since));
        }
    }

    pub fn open() {
        let mut state = state();
        if matches!(state.output, Output::Unopened) {
            state.output = open_output();
        }
    }

    fn open_output() -> Output {
        let Some(path) = std::env::var_os(PHASES_FILE_ENV) else {
            return Output::None;
        };
        match File::create(&path) {
            Ok(file) => Output::File(file),
            Err(error) => {
                eprintln!(
                    "warning: phases: can't create {}, so no phases are written: {error}",
                    std::path::Path::new(&path).display()
                );
                Output::None
            }
        }
    }

    pub fn finish() {
        mark(Phase::Exit);
        open();
        let mut state = state();
        let Some((_, wall)) = state.origin else {
            return;
        };
        let text = render(wall, &state.marks);
        if let Output::File(file) = &mut state.output
            && let Err(error) = file.write_all(text.as_bytes())
        {
            eprintln!("warning: phases: failed to write: {error}");
        }
    }

    /// The marks taken so far, for the tests.
    #[cfg(test)]
    pub fn marks() -> Vec<(Phase, u128)> {
        state().marks.clone()
    }
}

// Without the feature each is an empty `const fn`, so a default build
// compiles every call away.

/// Reads the clocks the marks are measured from. Call it first thing in the
/// embedder's entry; a second call changes nothing.
#[cfg(feature = "phases")]
pub fn start() {
    enabled::start();
}

/// Reads the clocks the marks are measured from: nothing without the
/// `phases` feature.
#[cfg(not(feature = "phases"))]
#[inline]
pub const fn start() {}

/// Records that `phase` ended now. Before [`start`], it records nothing.
#[cfg(feature = "phases")]
pub fn mark(phase: Phase) {
    enabled::mark(phase);
}

/// Records that `phase` ended now: nothing without the `phases` feature.
#[cfg(not(feature = "phases"))]
#[inline]
pub const fn mark(_phase: Phase) {}

/// Opens the file [`PHASES_FILE_ENV`] names, for [`finish`] to write to.
///
/// A process calls it before it confines itself, after which no new file
/// opens. Unset, nothing is written; a file that can't be created is a
/// warning on stderr. Once opened, or found unset, later calls change
/// nothing.
#[cfg(feature = "phases")]
pub fn open() {
    enabled::open();
}

/// Opens the phases file: nothing without the `phases` feature.
#[cfg(not(feature = "phases"))]
#[inline]
pub const fn open() {}

/// Marks [`Phase::Exit`] and writes every mark to the file [`open`] opened,
/// opening it first when nothing has. Call it last, just before the
/// embedder's entry returns.
#[cfg(feature = "phases")]
pub fn finish() {
    enabled::finish();
}

/// Marks the exit and writes the marks: nothing without the `phases`
/// feature.
#[cfg(not(feature = "phases"))]
#[inline]
pub const fn finish() {}

#[cfg(test)]
mod tests {
    use super::*;

    /// The harness reads the file by its own copies of the contract, since
    /// it's independent of the runtime crates.
    #[test]
    fn the_harness_reads_the_file_this_writes() {
        assert_eq!(PHASES_FILE_ENV, gro_bench::phases::PHASES_FILE_ENV);
        assert_eq!(PHASES_HEADER, gro_bench::phases::PHASES_HEADER);
    }

    #[cfg(feature = "phases")]
    #[test]
    fn marks_are_measured_from_the_start_in_order() {
        mark(Phase::Engine);
        // nothing before the start
        assert!(enabled::marks().is_empty());
        start();
        mark(Phase::Engine);
        mark(Phase::Runtime);
        // other tests' runs mark too, on their own threads
        let marks = enabled::marks();
        let engine = marks.iter().position(|(phase, _)| *phase == Phase::Engine);
        let runtime = marks
            .iter()
            .rposition(|(phase, _)| *phase == Phase::Runtime);
        let (Some(engine), Some(runtime)) = (engine, runtime) else {
            panic!("{marks:?}");
        };
        assert!(engine < runtime, "{marks:?}");
        assert!(marks[engine].1 <= marks[runtime].1, "{marks:?}");
    }

    #[test]
    fn the_file_is_the_header_the_anchor_and_each_mark() {
        let text = render(17, &[(Phase::Engine, 5), (Phase::Exit, 1_000_000_007)]);
        assert_eq!(
            text,
            "gro_phases 1\nmain_unix_ns 17\nengine 5\nexit 1000000007\n"
        );
    }
}
