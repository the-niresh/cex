//! Latency samples and what gets published from them.
//!
//! The summary is convenience. The CSV is the point: a reader who cannot
//! recompute the percentiles from the raw buckets has to take the summary on
//! trust, and a number nobody can re-derive is a decoration.

use std::io::{self, Write};

use hdrhistogram::Histogram;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    pub count: u64,
    pub p50: u64,
    pub p90: u64,
    pub p99: u64,
    pub p999: u64,
    pub max: u64,
}

pub struct Samples {
    hist: Histogram<u64>,
}

impl Samples {
    pub fn new() -> Self {
        // 1µs to 60s, three significant figures. Audited: the bounds are valid
        // so the unwrap cannot fire.
        Samples {
            hist: Histogram::new_with_bounds(1, 60_000_000, 3).unwrap(),
        }
    }

    /// Values above the ceiling are clamped rather than dropped. A request that
    /// took longer than a minute is a real and very bad sample, and silently
    /// discarding it would improve every percentile.
    pub fn record(&mut self, micros: u64) {
        self.hist.saturating_record(micros.max(1));
    }

    pub fn summary(&self) -> Summary {
        Summary {
            count: self.hist.len(),
            p50: self.hist.value_at_quantile(0.50),
            p90: self.hist.value_at_quantile(0.90),
            p99: self.hist.value_at_quantile(0.99),
            p999: self.hist.value_at_quantile(0.999),
            max: self.hist.max(),
        }
    }

    pub fn write_csv(&self, mut w: impl Write) -> io::Result<()> {
        writeln!(w, "micros,count")?;
        for bucket in self.hist.iter_recorded() {
            writeln!(
                w,
                "{},{}",
                bucket.value_iterated_to(),
                bucket.count_at_value()
            )?;
        }
        Ok(())
    }
}

impl Default for Samples {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(values: &[u64]) -> Samples {
        let mut s = Samples::new();
        for v in values {
            s.record(*v);
        }
        s
    }

    #[test]
    fn percentiles_over_a_known_set() {
        // 1..=100, so p50 is 50 and p99 is 99, within the histogram's
        // configured precision.
        let values: Vec<u64> = (1..=100).collect();
        let summary = with(&values).summary();

        assert_eq!(summary.count, 100);
        assert_eq!(summary.p50, 50);
        assert_eq!(summary.p99, 99);
        assert_eq!(summary.max, 100);
    }

    #[test]
    fn an_empty_set_reports_zeroes_rather_than_panicking() {
        let summary = Samples::new().summary();
        assert_eq!(summary.count, 0);
        assert_eq!(summary.p50, 0);
    }

    #[test]
    fn the_csv_carries_every_bucket_so_percentiles_can_be_recomputed() {
        let mut buf = Vec::new();
        with(&[10, 10, 20, 30]).write_csv(&mut buf).unwrap();
        let text = String::from_utf8(buf).unwrap();

        assert!(text.starts_with("micros,count\n"), "got: {text}");
        // A summary a reader cannot recompute is a decoration. The total of
        // every count column must equal the number of samples recorded.
        let total: u64 = text
            .lines()
            .skip(1)
            .map(|l| l.split(',').nth(1).unwrap().parse::<u64>().unwrap())
            .sum();
        assert_eq!(total, 4);
    }
}
