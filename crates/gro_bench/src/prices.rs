//! The price table of a prices suite's run: what each of the default
//! runtime's defaults costs, as the default's median less that of the
//! variant with the default turned off.
//!
//! A prices suite ([`Prices`]) names its default runtime; every other
//! runtime is a variant of it, named for the default it turns off. Per
//! workload, each variant's price is the default's median less its own: of
//! wall time in a wall run, and of user-space and kernel-space instructions
//! in a counter run, whose wall times carry `perf`'s own and aren't priced.
//! A positive price is what keeping the default costs.
//!
//! A price is within its rows' noise (`~`) when it's no larger than the two
//! rows' median absolute deviations added: each MAD is its row's spread
//! sample to sample, so a difference of medians inside their sum is one the
//! run's samples can't tell from nothing. That is per row, unlike the A/A
//! noise row, which measures only the first row, and it is cautious: the
//! median's own error, about 1.86 MAD over the root of the sample count, is
//! below a MAD from four samples on.

use std::fmt::Write as _;

use crate::record::{Metric, Mode, RowRecord, RunRecord};
use crate::suite::Prices;

/// One measure's price: the default's median less the variant's, and the
/// two rows' MADs added, within which it's noise.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Priced {
    pub price: f64,
    pub noise: f64,
}

impl Priced {
    fn of(default: &Metric, variant: &Metric) -> Self {
        Self {
            price: default.stats.median - variant.stats.median,
            noise: default.stats.mad + variant.stats.mad,
        }
    }

    /// Whether the price is within its rows' noise.
    fn is_noise(self) -> bool {
        self.price.abs() <= self.noise
    }
}

/// One variant's price on one workload.
#[derive(Debug, Clone, PartialEq)]
pub struct Price {
    /// The variant runtime, named for the default it turns off.
    pub variant: String,
    pub workload: String,
    /// Of the median wall time, in nanoseconds.
    pub wall_ns: Priced,
    /// Of the median user-space instructions, in a counter run.
    pub instructions: Option<Priced>,
    /// Of the median kernel-space instructions, in a counter run.
    pub kernel_instructions: Option<Priced>,
}

/// Each variant's price on each workload, workload by workload, in the
/// record's row order; a row whose default row is missing is skipped.
pub fn prices(record: &RunRecord, prices: &Prices) -> Vec<Price> {
    let priced = |default: Option<&Metric>, variant: Option<&Metric>| {
        default.zip(variant).map(|(d, v)| Priced::of(d, v))
    };
    record
        .rows
        .iter()
        .filter(|row| row.runtime != prices.default)
        .filter_map(|row| {
            let default = default_row(record, prices, &row.workload)?;
            Some(Price {
                variant: row.runtime.clone(),
                workload: row.workload.clone(),
                wall_ns: Priced::of(&default.wall_ns, &row.wall_ns),
                instructions: priced(default.instructions.as_ref(), row.instructions.as_ref()),
                kernel_instructions: priced(
                    default.kernel_instructions.as_ref(),
                    row.kernel_instructions.as_ref(),
                ),
            })
        })
        .collect()
}

/// The default's row on `workload`.
fn default_row<'a>(
    record: &'a RunRecord,
    prices: &Prices,
    workload: &str,
) -> Option<&'a RowRecord> {
    record.row(&prices.default, workload)
}

/// A price and its share of the default's median, `~` marking noise.
fn cells(
    priced: Option<Priced>,
    default: Option<&Metric>,
    scale: f64,
    decimals: usize,
) -> (String, String) {
    let Some(priced) = priced else {
        return ("-".to_owned(), "-".to_owned());
    };
    let mark = if priced.is_noise() { "~" } else { " " };
    let share = default.map_or_else(
        || "-".to_owned(),
        |default| format!("{:.2}%", priced.price / default.stats.median * 100.0),
    );
    (format!("{:.decimals$}{mark}", priced.price / scale), share)
}

/// The price table, as printed after a prices suite's run.
///
/// Per workload, the default's medians, then each variant's prices with their share of
/// the default's, `~` marking one within its rows' noise. A wall run prices
/// wall time; a counter run prices user and kernel instructions only.
pub fn table(record: &RunRecord, prices_of: &Prices) -> String {
    let mut out = String::new();
    let width = record
        .rows
        .iter()
        .map(|row| row.runtime.len())
        .fold("default off".len(), usize::max);
    let counted = record.mode == Mode::Counters;
    if counted {
        let _ = writeln!(
            out,
            "{:<width$} {:<16} {:>14} {:>8} {:>14} {:>8}",
            "default off", "workload", "user instr", "% user", "kernel instr", "% kernel"
        );
    } else {
        let _ = writeln!(
            out,
            "{:<width$} {:<16} {:>10} {:>8}",
            "default off", "workload", "price ms", "% wall"
        );
    }
    let mut workload = None;
    for price in prices(record, prices_of) {
        let Some(default) = default_row(record, prices_of, &price.workload) else {
            continue;
        };
        let median = |metric: Option<&Metric>| {
            metric.map_or_else(|| "-".to_owned(), |m| format!("{:.0}", m.stats.median))
        };
        if workload.as_ref() != Some(&price.workload) {
            workload = Some(price.workload.clone());
            let label = format!("({})", prices_of.default);
            if counted {
                let _ = writeln!(
                    out,
                    "{label:<width$} {:<16} {:>14} {:>8} {:>14} {:>8}",
                    price.workload,
                    median(default.instructions.as_ref()),
                    "",
                    median(default.kernel_instructions.as_ref()),
                    "",
                );
            } else {
                let _ = writeln!(
                    out,
                    "{label:<width$} {:<16} {:>10.3} {:>8}",
                    price.workload,
                    default.wall_ns.stats.median / 1e6,
                    "",
                );
            }
        }
        if counted {
            let (user, user_share) =
                cells(price.instructions, default.instructions.as_ref(), 1.0, 0);
            let (kernel, kernel_share) = cells(
                price.kernel_instructions,
                default.kernel_instructions.as_ref(),
                1.0,
                0,
            );
            let _ = writeln!(
                out,
                "{:<width$} {:<16} {user:>14} {user_share:>8} {kernel:>14} {kernel_share:>8}",
                price.variant, price.workload,
            );
        } else {
            let (wall, share) = cells(Some(price.wall_ns), Some(&default.wall_ns), 1e6, 3);
            let _ = writeln!(
                out,
                "{:<width$} {:<16} {wall:>10} {share:>8}",
                price.variant, price.workload,
            );
        }
    }
    let _ = writeln!(
        out,
        "(each price is the default's median less the variant's, with that default off: what keeping it costs; `~` marks one within the two rows' MADs added{})",
        if counted {
            "; wall times aren't priced under perf. User instructions count what the process runs, kernel ones its system calls and faults: seccomp's user price is mostly compiling its BPF filters, once per eval, and its kernel price the filters run on each call"
        } else {
            ""
        },
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::{NoiseRecord, SuiteId};
    use crate::stats::Stats;
    use crate::test_support::{self, run_record};

    /// Samples `value - spread` and `value + spread`: median `value`, MAD
    /// `spread`.
    fn metric(value: u64, spread: u64) -> Metric {
        let samples = vec![value - spread, value + spread];
        Metric {
            stats: Stats::from_samples(&samples).unwrap(),
            samples,
        }
    }

    fn row(runtime: &str, wall: u64, instructions: u64, spread: u64) -> RowRecord {
        RowRecord {
            instructions: Some(metric(instructions, spread)),
            kernel_instructions: Some(metric(instructions / 2, spread)),
            ..test_support::row(runtime, "w", metric(wall, 1000))
        }
    }

    fn record(mode: Mode) -> RunRecord {
        let base = run_record(vec![
            row("fgro", 10_000_000, 1_000_000, 10),
            row("box", 9_000_000, 900_000, 10),
            // a price of 100 within MADs of 10 and 200
            row("locale", 10_000_100, 1_000_100, 200),
        ]);
        RunRecord {
            mode,
            suite: SuiteId {
                name: "prices".to_owned(),
                version: 1,
            },
            // the A/A row (the first's wall time): its deviation would call
            // every price here noise
            noise: NoiseRecord {
                median_deviation: 0.5,
                instructions_median_deviation: Some(0.5),
                ..base.noise
            },
            ..base
        }
    }

    #[test]
    fn a_price_is_the_default_less_the_variant_within_its_rows_noise() {
        let defaults = Prices {
            default: "fgro".to_owned(),
        };
        let counted = record(Mode::Counters);
        let found = prices(&counted, &defaults);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].variant, "box");
        assert!((found[0].wall_ns.price - 1_000_000.0).abs() < 1e-6);
        let user = found[0].instructions.unwrap();
        assert!((user.price - 100_000.0).abs() < 1e-6 && (user.noise - 20.0).abs() < 1e-6);
        assert!(!user.is_noise());
        let locale = found[1].instructions.unwrap();
        assert!((locale.price + 100.0).abs() < 1e-6, "{locale:?}");
        assert!(locale.is_noise());

        let text = table(&counted, &defaults);
        let line = |name: &str| text.lines().find(|line| line.starts_with(name)).unwrap();
        assert!(!line("box").contains('~'), "{text}");
        assert!(line("locale").contains('~'), "{text}");
        // instructions only, user and kernel, under perf
        assert!(
            text.contains("kernel instr") && !text.contains("price ms"),
            "{text}"
        );
        assert!(line("box").contains("50000"), "{text}");
        // a wall run prices wall time alone
        let text = table(&record(Mode::Wall), &defaults);
        assert!(
            text.contains("price ms") && !text.contains("instr"),
            "{text}"
        );
        assert!(
            text.lines()
                .any(|line| line.starts_with("box") && line.contains("1.000 "))
        );
    }
}
