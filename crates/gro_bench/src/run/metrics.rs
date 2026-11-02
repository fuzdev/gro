//! The metrics of a row's samples, and the noise row's record.

use super::command::RowCommand;
use super::sample::Sample;
use crate::counters::Counts;
use crate::error::BenchError;
use crate::record::{Metric, NoiseRecord, PhaseMetric, RowRecord};
use crate::stats::{Stats, ratio_deviation};

/// The peak memory of every sample, when each has one.
pub(super) fn rss_metric(samples: &[Sample]) -> Option<Metric> {
    samples
        .iter()
        .map(|sample| sample.max_rss_kib)
        .collect::<Option<Vec<u64>>>()
        .map(metric)
}

pub(super) fn wall_metric(samples: &[Sample]) -> Metric {
    metric(samples.iter().map(|sample| sample.wall_ns).collect())
}

/// The metric of `samples`, which a row always has.
pub(super) fn metric(samples: Vec<u64>) -> Metric {
    #[expect(
        clippy::expect_used,
        reason = "every row gets `samples` samples, which the suite's type keeps above 0"
    )]
    let stats = Stats::from_samples(&samples).expect("a row has samples");
    Metric { stats, samples }
}

/// One count of every sample, when each was counted.
pub(super) fn count_metric(samples: &[Sample], count: impl Fn(&Counts) -> u64) -> Option<Metric> {
    samples
        .iter()
        .map(|sample| sample.counts.as_ref().map(&count))
        .collect::<Option<Vec<u64>>>()
        .map(metric)
}

/// Each phase's durations over `row`'s samples, which must name the same
/// phases in the same order.
pub(super) fn phase_metrics(
    row: &RowCommand,
    samples: &[Sample],
) -> Result<Vec<PhaseMetric>, BenchError> {
    let Some(first) = samples.first() else {
        return Ok(Vec::new());
    };
    let names: Vec<&str> = first.phases.iter().map(|(name, _)| name.as_str()).collect();
    for sample in samples {
        if !sample
            .phases
            .iter()
            .map(|(name, _)| name.as_str())
            .eq(names.iter().copied())
        {
            return Err(BenchError::Phases(format!(
                "the samples of `{}` marked different phases",
                row.display()
            )));
        }
    }
    Ok(names
        .iter()
        .enumerate()
        .map(|(index, name)| PhaseMetric {
            name: (*name).to_owned(),
            ns: metric(
                samples
                    .iter()
                    .map(|sample| sample.phases[index].1)
                    .collect(),
            ),
        })
        .collect())
}

pub(super) fn noise_record(first: &RowRecord, copy: &[Sample]) -> NoiseRecord {
    let wall_ns = wall_metric(copy);
    let instructions = count_metric(copy, |counts| counts.instructions);
    let instructions_median_deviation = first
        .instructions
        .as_ref()
        .zip(instructions.as_ref())
        .map(|(first, copy)| ratio_deviation(first.stats.median, copy.stats.median));
    NoiseRecord {
        runtime: first.runtime.clone(),
        workload: first.workload.clone(),
        median_deviation: ratio_deviation(first.wall_ns.stats.median, wall_ns.stats.median),
        wall_ns,
        instructions,
        instructions_median_deviation,
    }
}
