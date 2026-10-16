//! Timing statistics: MAD outlier cleaning and the figures a run records.
//!
//! The method is `@fuzdev/fuz_util`'s (`stats_outliers_mad` and
//! `BenchmarkStats`), so the harness cleans samples the way the ecosystem's
//! in-process benchmarks do:
//!
//! - an outlier is a sample whose modified z-score, `0.6745 * (x - median) /
//!   MAD`, exceeds 3.5;
//! - when that removes more than 3/10 of the samples, the bound widens to 5;
//! - when that still removes more than 4/10, the 4/5 of samples closest to the
//!   median are kept;
//! - when the MAD is 0, the IQR method decides instead (outside `1.5 * IQR`
//!   past the quartiles), and an IQR of 0 removes nothing;
//! - fewer than 3 samples are never cleaned.
//!
//! The ratio thresholds are compared exactly in integers, where the TS
//! compares `count > n * 0.3` in floating point; they can differ only at a
//! boundary the float product rounds across.
//!
//! The median and MAD describe every sample; the mean, cv and min describe the
//! cleaned ones. A low outlier is an invalid measurement, not a fast run, so
//! `min` never reports one.

use serde::{Deserialize, Serialize};

/// The modified z-score's scale, the MAD of a standard normal distribution.
const MAD_CONSTANT: f64 = 0.6745;
/// The modified z-score past which a sample is an outlier.
const Z_THRESHOLD: f64 = 3.5;
/// The wider bound used when [`Z_THRESHOLD`] removes too many samples.
const Z_EXTREME: f64 = 5.0;
/// How far past the quartiles, in IQRs, the IQR fallback's bounds sit.
const IQR_MULTIPLIER: f64 = 1.5;
/// Below this many samples nothing is cleaned.
const MIN_SAMPLES: usize = 3;

/// Summary statistics of one metric's samples, in the samples' unit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stats {
    /// How many samples were measured.
    pub n: usize,
    /// How many of them the MAD cleaning removed.
    pub outliers: usize,
    /// The median of every sample.
    pub median: f64,
    /// The median absolute deviation from `median`, over every sample,
    /// unscaled.
    pub mad: f64,
    /// The mean of the cleaned samples.
    pub mean: f64,
    /// The coefficient of variation of the cleaned samples: their sample
    /// standard deviation over their mean, 0 for a single sample.
    pub cv: f64,
    /// The smallest cleaned sample.
    pub min: f64,
}

impl Stats {
    /// Computes the statistics of `samples`, or `None` when there are none.
    pub fn from_samples(samples: &[u64]) -> Option<Self> {
        if samples.is_empty() {
            return None;
        }
        let values: Vec<f64> = samples.iter().copied().map(to_f64).collect();
        let median = median(&values);
        let mad = mad(&values, median);
        let Cleaned { cleaned, outliers } = clean(&values);
        // the cleaning always keeps the median's neighborhood, so `cleaned` is never empty
        let count = to_f64(cleaned.len() as u64);
        let mean = cleaned.iter().sum::<f64>() / count;
        let std_dev = if cleaned.len() < 2 {
            0.0
        } else {
            let squares: f64 = cleaned.iter().map(|value| (value - mean).powi(2)).sum();
            (squares / (count - 1.0)).sqrt()
        };
        let min = cleaned.iter().copied().fold(f64::INFINITY, f64::min);
        Some(Self {
            n: samples.len(),
            outliers: outliers.len(),
            median,
            mad,
            mean,
            cv: std_dev / mean,
            min,
        })
    }
}

/// How far apart two positive measurements of the same thing are, the same in
/// either direction: the larger over the smaller, minus 1.
pub fn ratio_deviation(a: f64, b: f64) -> f64 {
    a.max(b) / a.min(b) - 1.0
}

/// Samples split by [`clean`].
#[derive(Debug, PartialEq)]
pub(crate) struct Cleaned {
    pub cleaned: Vec<f64>,
    pub outliers: Vec<f64>,
}

/// Splits `values` into the samples kept and the outliers, by the module's
/// method.
pub(crate) fn clean(values: &[f64]) -> Cleaned {
    if values.len() < MIN_SAMPLES {
        return keep_all(values);
    }
    let median = median(values);
    let mad = mad(values, median);
    if mad == 0.0 {
        return clean_iqr(values);
    }
    let split = |threshold: f64| {
        let (outliers, cleaned) = values
            .iter()
            .copied()
            .partition(|&value| (MAD_CONSTANT * (value - median) / mad).abs() > threshold);
        Cleaned { cleaned, outliers }
    };
    let n = values.len();
    let first = split(Z_THRESHOLD);
    if first.outliers.len() * 10 <= n * 3 {
        return first;
    }
    let second = split(Z_EXTREME);
    if second.outliers.len() * 10 <= n * 4 {
        return second;
    }
    // too heavy a tail for any bound: keep the closest to the median (a stable sort, as the TS's)
    let mut by_distance = values.to_vec();
    by_distance.sort_by(|a, b| (a - median).abs().total_cmp(&(b - median).abs()));
    let outliers = by_distance.split_off(n * 4 / 5);
    Cleaned {
        cleaned: by_distance,
        outliers,
    }
}

/// The IQR fallback for a MAD of 0.
#[expect(
    clippy::suboptimal_flops,
    reason = "the bounds match the TS's unfused arithmetic"
)]
fn clean_iqr(values: &[f64]) -> Cleaned {
    let sorted = sorted(values);
    let n = sorted.len();
    let q1 = sorted[n / 4];
    let q3 = sorted[n * 3 / 4];
    let iqr = q3 - q1;
    if iqr == 0.0 {
        return keep_all(values);
    }
    // unfused, as the TS computes them
    let (lower, upper) = (q1 - IQR_MULTIPLIER * iqr, q3 + IQR_MULTIPLIER * iqr);
    let (outliers, cleaned) = values
        .iter()
        .copied()
        .partition(|&value| value < lower || value > upper);
    Cleaned { cleaned, outliers }
}

fn keep_all(values: &[f64]) -> Cleaned {
    Cleaned {
        cleaned: values.to_vec(),
        outliers: Vec::new(),
    }
}

fn sorted(values: &[f64]) -> Vec<f64> {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted
}

/// The median of a non-empty slice.
fn median(values: &[f64]) -> f64 {
    let sorted = sorted(values);
    let mid = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        f64::midpoint(sorted[mid - 1], sorted[mid])
    } else {
        sorted[mid]
    }
}

/// The median absolute deviation of a non-empty slice from its `median`.
fn mad(values: &[f64], median: f64) -> f64 {
    let deviations: Vec<f64> = values.iter().map(|value| (value - median).abs()).collect();
    self::median(&deviations)
}

/// Converts a sample or a count to `f64`, exactly below 2^53 (a nanosecond
/// sample of over a hundred days).
#[expect(
    clippy::cast_precision_loss,
    reason = "samples and counts stay far below 2^53"
)]
const fn to_f64(value: u64) -> f64 {
    value as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(actual: f64, expected: f64) {
        let tolerance = expected.abs() * 1e-12;
        assert!(
            (actual - expected).abs() <= tolerance,
            "{actual} is not {expected}"
        );
    }

    fn assert_clean(values: &[f64], cleaned: &[f64], outliers: &[f64]) {
        let mut result = clean(values);
        result.cleaned.sort_by(f64::total_cmp);
        result.outliers.sort_by(f64::total_cmp);
        assert_eq!(result.cleaned, sorted(cleaned), "cleaned");
        assert_eq!(result.outliers, sorted(outliers), "outliers");
    }

    /// Asserts `Stats` for `samples` against the figures `fuz_util`'s TS
    /// computes for the same input.
    fn assert_stats(samples: &[u64], expected: &Stats) {
        let stats = Stats::from_samples(samples).unwrap();
        assert_eq!(stats.n, expected.n, "n");
        assert_eq!(stats.outliers, expected.outliers, "outliers");
        assert_close(stats.median, expected.median);
        assert_close(stats.mad, expected.mad);
        assert_close(stats.mean, expected.mean);
        assert_close(stats.cv, expected.cv);
        assert_close(stats.min, expected.min);
    }

    // The expected splits and figures below are `@fuzdev/fuz_util`'s
    // `stats_outliers_mad`, `stats_median`, `stats_mean` and
    // `stats_std_dev_sample` run on the same inputs.

    #[test]
    fn no_outliers() {
        let values = [10.0, 11.0, 12.0, 13.0, 14.0];
        assert_clean(&values, &values, &[]);
    }

    #[test]
    fn high_outliers() {
        assert_clean(
            &[10.0, 11.0, 12.0, 13.0, 14.0, 100.0, 200.0],
            &[10.0, 11.0, 12.0, 13.0, 14.0],
            &[100.0, 200.0],
        );
    }

    #[test]
    fn low_outlier() {
        assert_clean(
            &[100.0, 101.0, 102.0, 103.0, 104.0, 105.0, 1.0],
            &[100.0, 101.0, 102.0, 103.0, 104.0, 105.0],
            &[1.0],
        );
    }

    #[test]
    fn widens_to_the_extreme_bound() {
        // 4/11 past 3.5 is more than 3/10; 3/11 past 5 isn't more than 4/10
        assert_clean(
            &[
                103.0, 103.0, 100.0, 155.0, 125.0, 155.0, 158.0, 102.0, 102.0, 116.0, 101.0,
            ],
            &[103.0, 103.0, 100.0, 125.0, 102.0, 102.0, 116.0, 101.0],
            &[155.0, 155.0, 158.0],
        );
    }

    #[test]
    fn keeps_the_closest() {
        // more than 4/10 past 5, so the 4/5 closest to the median stay
        assert_clean(
            &[
                137.0, 100.0, 102.0, 127.0, 101.0, 102.0, 155.0, 102.0, 100.0, 135.0, 100.0, 159.0,
            ],
            &[
                102.0, 102.0, 102.0, 101.0, 100.0, 100.0, 100.0, 127.0, 135.0,
            ],
            &[137.0, 155.0, 159.0],
        );
    }

    #[test]
    fn zero_mad_falls_back_to_iqr() {
        assert_clean(
            &[5.0, 5.0, 5.0, 5.0, 6.0, 6.0, 100.0],
            &[5.0, 5.0, 5.0, 5.0, 6.0, 6.0],
            &[100.0],
        );
    }

    #[test]
    fn zero_mad_and_zero_iqr_keep_everything() {
        let values = [5.0, 5.0, 5.0, 5.0, 100.0];
        assert_clean(&values, &values, &[]);
    }

    #[test]
    fn too_few_to_clean() {
        let values = [1.0, 1000.0];
        assert_clean(&values, &values, &[]);
    }

    #[test]
    fn stats_of_clean_samples() {
        assert_stats(
            &[10, 11, 12, 13, 14],
            &Stats {
                n: 5,
                outliers: 0,
                median: 12.0,
                mad: 1.0,
                mean: 12.0,
                cv: 0.131_761_569_173_682_48,
                min: 10.0,
            },
        );
    }

    #[test]
    fn stats_describe_every_sample_or_the_cleaned_ones() {
        // the median and MAD include the outliers; the mean, cv and min don't
        assert_stats(
            &[10, 11, 12, 13, 14, 100, 200],
            &Stats {
                n: 7,
                outliers: 2,
                median: 13.0,
                mad: 2.0,
                mean: 12.0,
                cv: 0.131_761_569_173_682_48,
                min: 10.0,
            },
        );
        // a low outlier never becomes the min
        assert_stats(
            &[100, 101, 102, 103, 104, 105, 1],
            &Stats {
                n: 7,
                outliers: 1,
                median: 102.0,
                mad: 2.0,
                mean: 102.5,
                cv: 0.018_251_987_252_555_81,
                min: 100.0,
            },
        );
        assert_stats(
            &[137, 100, 102, 127, 101, 102, 155, 102, 100, 135, 100, 159],
            &Stats {
                n: 12,
                outliers: 3,
                median: 102.0,
                mad: 2.0,
                mean: 107.666_666_666_666_67,
                cv: 0.124_524_034_159_058_1,
                min: 100.0,
            },
        );
    }

    #[test]
    fn stats_of_one_sample() {
        assert_eq!(
            Stats::from_samples(&[42]),
            Some(Stats {
                n: 1,
                outliers: 0,
                median: 42.0,
                mad: 0.0,
                mean: 42.0,
                cv: 0.0,
                min: 42.0,
            })
        );
    }

    #[test]
    fn stats_of_no_samples() {
        assert_eq!(Stats::from_samples(&[]), None);
    }

    #[test]
    fn ratio_deviation_is_symmetric() {
        assert_close(ratio_deviation(110.0, 100.0), 0.1);
        assert_close(ratio_deviation(100.0, 110.0), 0.1);
        assert_close(ratio_deviation(7.0, 7.0), 0.0);
    }
}
