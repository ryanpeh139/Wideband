//! Scanner histograms (HP Tuners VCM Scanner style): bin any logged channel into a table's
//! cells by two axis channels, with an optional filter, then copy/paste into the editor.
//!
//! Samples go to the *nearest* cell (scanner histogram convention), and samples beyond the
//! axis breakpoints are dropped rather than piled onto the edge cells.

use crate::live_trace::axis_position;
use crate::ve_correction::SampleSource;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Aggregate {
    Mean,
    Min,
    Max,
    Count,
}

impl Aggregate {
    pub const ALL: [Aggregate; 4] = [Aggregate::Mean, Aggregate::Min, Aggregate::Max, Aggregate::Count];

    pub fn label(self) -> &'static str {
        match self {
            Aggregate::Mean => "Average",
            Aggregate::Min => "Minimum",
            Aggregate::Max => "Maximum",
            Aggregate::Count => "Hit count",
        }
    }
}

/// Keep only samples where `channel` is within `[min, max]` (e.g. ECT 70..120).
#[derive(Clone, Debug, PartialEq)]
pub struct Filter {
    pub channel: String,
    pub min: f64,
    pub max: f64,
}

#[derive(Clone, Debug)]
pub struct HistogramSpec {
    pub value_channel: String,
    pub x_channel: String,
    pub y_channel: String,
    pub aggregate: Aggregate,
    pub filter: Option<Filter>,
    /// Bins with fewer hits are left empty.
    pub min_hits: u32,
}

#[derive(Clone, Copy, Debug, Default)]
struct Bin {
    n: u32,
    sum: f64,
    min: f64,
    max: f64,
}

#[derive(Clone, Debug)]
pub struct Histogram {
    pub spec: HistogramSpec,
    /// Table this histogram was built against (axes define the bins).
    pub table_id: String,
    bins: Vec<Vec<Bin>>,
    pub used: usize,
    pub filtered: usize,
    pub off_axes: usize,
    pub missing: usize,
}

impl Histogram {
    pub fn rows(&self) -> usize {
        self.bins.len()
    }

    pub fn cols(&self) -> usize {
        self.bins.first().map_or(0, |r| r.len())
    }

    pub fn hits(&self, r: usize, c: usize) -> u32 {
        self.bins.get(r).and_then(|row| row.get(c)).map_or(0, |b| b.n)
    }

    pub fn value(&self, r: usize, c: usize) -> Option<f64> {
        let b = self.bins.get(r)?.get(c)?;
        if b.n == 0 || b.n < self.spec.min_hits {
            return None;
        }
        Some(match self.spec.aggregate {
            Aggregate::Mean => b.sum / b.n as f64,
            Aggregate::Min => b.min,
            Aggregate::Max => b.max,
            Aggregate::Count => b.n as f64,
        })
    }

    pub fn grid(&self) -> Vec<Vec<Option<f64>>> {
        (0..self.rows()).map(|r| (0..self.cols()).map(|c| self.value(r, c)).collect()).collect()
    }

    pub fn range(&self) -> Option<(f64, f64)> {
        let vals: Vec<f64> = self.grid().into_iter().flatten().flatten().collect();
        if vals.is_empty() {
            return None;
        }
        Some((
            vals.iter().cloned().fold(f64::MAX, f64::min),
            vals.iter().cloned().fold(f64::MIN, f64::max),
        ))
    }
}

/// Nearest-cell index for `v` along `labels`, or `None` when `v` is outside the breakpoints.
fn bin_index(labels: &[f64], v: f64) -> Option<usize> {
    if labels.len() < 2 {
        return None;
    }
    let lo = labels.iter().cloned().fold(f64::MAX, f64::min);
    let hi = labels.iter().cloned().fold(f64::MIN, f64::max);
    if v < lo || v > hi {
        return None;
    }
    axis_position(labels, v).map(|p| p.round() as usize)
}

pub fn build(
    table_id: &str,
    x_labels: &[f64],
    y_labels: &[f64],
    src: &dyn SampleSource,
    spec: &HistogramSpec,
) -> Histogram {
    let mut bins = vec![vec![Bin::default(); x_labels.len()]; y_labels.len()];
    let (mut used, mut filtered, mut off_axes, mut missing) = (0, 0, 0, 0);
    for i in 0..src.len() {
        if let Some(f) = &spec.filter {
            match src.value(i, &f.channel) {
                Some(v) if v >= f.min && v <= f.max => {}
                Some(_) => {
                    filtered += 1;
                    continue;
                }
                None => {
                    missing += 1;
                    continue;
                }
            }
        }
        let (Some(x), Some(y), Some(v)) = (
            src.value(i, &spec.x_channel),
            src.value(i, &spec.y_channel),
            src.value(i, &spec.value_channel),
        ) else {
            missing += 1;
            continue;
        };
        let (Some(c), Some(r)) = (bin_index(x_labels, x), bin_index(y_labels, y)) else {
            off_axes += 1;
            continue;
        };
        let b = &mut bins[r][c];
        if b.n == 0 {
            b.min = v;
            b.max = v;
        } else {
            b.min = b.min.min(v);
            b.max = b.max.max(v);
        }
        b.n += 1;
        b.sum += v;
        used += 1;
    }
    Histogram {
        spec: spec.clone(),
        table_id: table_id.to_string(),
        bins,
        used,
        filtered,
        off_axes,
        missing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Rows(Vec<HashMap<&'static str, f64>>);

    impl SampleSource for Rows {
        fn len(&self) -> usize {
            self.0.len()
        }
        fn time(&self, i: usize) -> f64 {
            i as f64 * 0.05
        }
        fn value(&self, i: usize, ch: &str) -> Option<f64> {
            self.0[i].get(ch).copied()
        }
        fn has_channel(&self, ch: &str) -> bool {
            self.0.iter().any(|m| m.contains_key(ch))
        }
    }

    fn row(rpm: f64, map: f64, timing: f64, ect: f64) -> HashMap<&'static str, f64> {
        [("rpm", rpm), ("map", map), ("timing", timing), ("ect", ect)].into_iter().collect()
    }

    fn spec(agg: Aggregate) -> HistogramSpec {
        HistogramSpec {
            value_channel: "timing".into(),
            x_channel: "rpm".into(),
            y_channel: "map".into(),
            aggregate: agg,
            filter: None,
            min_hits: 1,
        }
    }

    const RPM: [f64; 3] = [1000.0, 2000.0, 3000.0];
    const MAP: [f64; 2] = [40.0, 80.0];

    #[test]
    fn bins_to_nearest_cell_and_aggregates() {
        let src = Rows(vec![row(1900.0, 75.0, 20.0, 90.0), row(2100.0, 79.0, 24.0, 90.0), row(1000.0, 40.0, 10.0, 90.0)]);
        let h = build("t", &RPM, &MAP, &src, &spec(Aggregate::Mean));
        assert_eq!(h.value(1, 1), Some(22.0));
        assert_eq!(h.value(0, 0), Some(10.0));
        assert_eq!(h.value(0, 2), None);
        assert_eq!(h.hits(1, 1), 2);
        assert_eq!(build("t", &RPM, &MAP, &src, &spec(Aggregate::Max)).value(1, 1), Some(24.0));
        assert_eq!(build("t", &RPM, &MAP, &src, &spec(Aggregate::Count)).value(1, 1), Some(2.0));
    }

    #[test]
    fn off_axis_samples_are_dropped_not_clamped() {
        let src = Rows(vec![row(6000.0, 60.0, 30.0, 90.0), row(2000.0, 120.0, 30.0, 90.0)]);
        let h = build("t", &RPM, &MAP, &src, &spec(Aggregate::Mean));
        assert_eq!(h.off_axes, 2);
        assert_eq!(h.used, 0);
    }

    #[test]
    fn filter_and_min_hits() {
        let src = Rows(vec![row(2000.0, 80.0, 20.0, 40.0), row(2000.0, 80.0, 24.0, 90.0)]);
        let mut s = spec(Aggregate::Mean);
        s.filter = Some(Filter { channel: "ect".into(), min: 70.0, max: 130.0 });
        let h = build("t", &RPM, &MAP, &src, &s);
        assert_eq!((h.filtered, h.used), (1, 1));
        assert_eq!(h.value(1, 1), Some(24.0));
        s.min_hits = 2;
        assert_eq!(build("t", &RPM, &MAP, &src, &s).value(1, 1), None);
    }
}
