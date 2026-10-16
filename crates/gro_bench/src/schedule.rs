//! The order a run takes its samples in.
//!
//! A run never measures one row in a batch. It goes in rounds, and each round
//! takes one sample of every row. Round `r` starts from the rows rotated left
//! by `r / 2`, then runs them forward or reversed: reversed when `r` has an
//! odd number of set bits (the Thue–Morse sequence).
//!
//! Rounds `2k` and `2k + 1` share a rotation and run in opposite directions,
//! so each pair is a palindrome and a drift linear in time (a machine warming,
//! a background job ramping up) adds the same total to every row at every even
//! sample count, whatever the rows' durations. That holds for a row's mean;
//! for its median it holds only approximately. No row keeps a fixed position,
//! either: each sits early in some rounds and late in others.
//!
//! A pair's turn point runs one row twice back to back, which the rotation
//! moves from row to row, so no row keeps the warm repeat to itself.

/// The order of a run's samples: row indexes, where row `i` appears
/// `counts[i]` times.
///
/// Round `r` takes every row whose count is above `r`, rotated left by
/// `r / 2`, in the direction its Thue–Morse bit gives. The timed samples give
/// every row the same count; warmups can differ per row, and a row drops out
/// once its count is reached.
pub fn schedule(counts: &[usize]) -> Vec<usize> {
    let rounds = counts.iter().copied().max().unwrap_or(0);
    let mut order = Vec::with_capacity(counts.iter().sum());
    for round in 0..rounds {
        let start = order.len();
        order.extend((0..counts.len()).filter(|&row| counts[row] > round));
        let taken = &mut order[start..];
        // a round has at least one row: the longest count is above it
        taken.rotate_left((round / 2) % taken.len());
        if round.count_ones() % 2 == 1 {
            taken.reverse();
        }
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_rows_go_abba_abba_baab_baab() {
        // A = 0, B = 1
        assert_eq!(
            schedule(&[8, 8]),
            [
                0, 1, 1, 0, 0, 1, 1, 0, // ABBA ABBA
                1, 0, 0, 1, 1, 0, 0, 1, // BAAB BAAB
            ]
        );
    }

    #[test]
    fn rounds_rotate_by_pair_and_alternate_direction() {
        assert_eq!(schedule(&[4, 4, 4]), [0, 1, 2, 2, 1, 0, 0, 2, 1, 1, 2, 0]);
    }

    /// Each row's mean sample midpoint, in time units, when row `i` takes
    /// `durations[i]` per sample: what a linear drift adds to its mean.
    fn mean_midpoints(order: &[usize], durations: &[u64]) -> Vec<f64> {
        let mut sums = vec![0; durations.len()];
        let mut counts = vec![0_u32; durations.len()];
        let mut time = 0;
        for &row in order {
            // doubled, so a midpoint stays an integer
            sums[row] += 2 * time + durations[row];
            counts[row] += 1;
            time += durations[row];
        }
        #[expect(clippy::cast_precision_loss, reason = "test sums are small")]
        sums.iter()
            .zip(&counts)
            .map(|(&sum, &count)| sum as f64 / 2.0 / f64::from(count))
            .collect()
    }

    #[test]
    fn linear_drift_is_balanced_at_every_even_count() {
        for rows in 1..=7_usize {
            let equal = vec![1; rows];
            // durations that differ per row, as real rows' do
            let unequal: Vec<u64> = (0..7).map(|row| 10 + 37 * row).take(rows).collect();
            for samples in (2..=16).step_by(2) {
                let order = schedule(&vec![samples; rows]);
                for durations in [&equal, &unequal] {
                    let means = mean_midpoints(&order, durations);
                    #[expect(clippy::float_cmp, reason = "the balance is exact")]
                    let balanced = means.iter().all(|&mean| mean == means[0]);
                    assert!(
                        balanced,
                        "{rows} rows, {samples} samples, durations {durations:?}: {means:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn back_to_back_repeats_spread_across_rows() {
        for rows in 2..=7 {
            // two full rotations
            let order = schedule(&vec![4 * rows; rows]);
            let mut repeats = vec![0; rows];
            for pair in order.windows(2) {
                if pair[0] == pair[1] {
                    repeats[pair[0]] += 1;
                }
            }
            let fewest = repeats.iter().min().copied().unwrap_or(0);
            let most = repeats.iter().max().copied().unwrap_or(0);
            // without the rotation, every repeat lands on the first and last rows
            assert!(
                fewest >= 1 && most <= 2 * fewest,
                "{rows} rows: {repeats:?}"
            );
        }
    }

    #[test]
    fn each_row_appears_its_count() {
        let counts = [3, 0, 1, 5];
        let order = schedule(&counts);
        for (row, &count) in counts.iter().enumerate() {
            assert_eq!(order.iter().filter(|&&r| r == row).count(), count);
        }
        // rows drop out as their counts are reached
        assert_eq!(order, [0, 2, 3, 3, 0, 0, 3, 3, 3]);
    }

    #[test]
    fn nothing_to_schedule() {
        assert!(schedule(&[]).is_empty());
        assert!(schedule(&[0, 0]).is_empty());
    }
}
