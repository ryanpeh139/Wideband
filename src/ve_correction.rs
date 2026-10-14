//! VE / fuel-table correction from logged lambda and fuel trims (HP Tuners "VE from wideband"
//! style). Pure analysis: given log samples and a table, produce per-cell correction factors.
//!
//! Per sample the fuelling error is
//!   ratio = (lambda_measured / lambda_target) * (1 + STFT/100) * (1 + LTFT/100)
//! which covers open loop (wideband shows the error) and closed loop (trims hold the error).
//! `ratio > 1` = engine ran lean = the table must go *up* by that factor. Samples are spread over
//! the 4 surrounding cells with bilinear weights — the same blend the ECU uses on lookup.
//!
//! Only meaningful for tables where a larger value means more fuel (VE, fuel mass, injector
//! base). Never auto-applied: results are suggestions the user applies to a selection (undoable).

use crate::definitions::XdfTable;
use crate::live_trace::table_position;
use std::collections::HashMap;

/// Window for the throttle-rate (transient) filter.
const TPS_RATE_WINDOW_S: f64 = 0.2;

/// Where the commanded lambda comes from.
#[derive(Clone, Debug)]
pub enum TargetLambda {
    /// Fixed target (e.g. 1.0 for a stoich closed-loop region).
    Constant(f64),
    /// Commanded-lambda table from the XDF, looked up at each sample's operating point.
    Table(XdfTable),
}

#[derive(Clone, Debug)]
pub struct CorrectionSettings {
    pub target: TargetLambda,
    /// Reject idle / cranking.
    pub min_rpm: f64,
    /// Reject cold engine (warm-up enrichment skews lambda). Ignored if `ect` isn't logged.
    pub min_ect_c: f64,
    /// Plausible measured lambda window (rejects fuel cut, sensor rail, misfire).
    pub lambda_min: f64,
    pub lambda_max: f64,
    /// Throttle rate above which a sample is a transient (accel enrichment / wall wetting).
    /// Ignored if `tps` isn't logged.
    pub max_tps_rate_pct_s: f64,
    /// Lambda sensor transport delay: pair lambda at t with operating point at t - delay.
    pub lambda_delay_s: f64,
    /// Minimum accumulated bilinear weight before a cell gets a suggestion.
    pub min_weight: f64,
    /// Clamp per pass so one bad log can't wreck the table; iterate log -> apply -> log.
    pub max_change_pct: f64,
}

impl Default for CorrectionSettings {
    fn default() -> Self {
        Self {
            target: TargetLambda::Constant(1.0),
            min_rpm: 600.0,
            min_ect_c: 70.0,
            lambda_min: 0.6,
            lambda_max: 1.35,
            max_tps_rate_pct_s: 40.0,
            lambda_delay_s: 0.15,
            min_weight: 5.0,
            max_change_pct: 15.0,
        }
    }
}

/// Log samples: time-ordered, values looked up by channel id.
pub trait SampleSource {
    fn len(&self) -> usize;
    fn time(&self, i: usize) -> f64;
    fn value(&self, i: usize, channel: &str) -> Option<f64>;
    fn has_channel(&self, channel: &str) -> bool;
}

/// Telemetry frames + channel index (live recording or an imported CSV log).
///
/// CSV import fills missing columns with 0.0, and a channel the ECU never answers stays 0 in a
/// live recording. A channel counts as logged only if it is non-zero somewhere — otherwise a log
/// without ECT would read as 0 °C and every sample would be rejected as "cold".
pub struct FrameSource<'a> {
    index: &'a HashMap<String, usize>,
    frames: &'a [crate::telemetry::TelemetryFrame],
    present: std::collections::HashSet<usize>,
}

impl<'a> FrameSource<'a> {
    pub fn new(index: &'a HashMap<String, usize>, frames: &'a [crate::telemetry::TelemetryFrame]) -> Self {
        let present = index
            .values()
            .copied()
            .filter(|&idx| frames.iter().any(|f| f.values.get(idx).map_or(false, |v| *v != 0.0)))
            .collect();
        Self { index, frames, present }
    }
}

impl SampleSource for FrameSource<'_> {
    fn len(&self) -> usize {
        self.frames.len()
    }
    fn time(&self, i: usize) -> f64 {
        self.frames[i].time_sec
    }
    fn value(&self, i: usize, channel: &str) -> Option<f64> {
        let idx = *self.index.get(channel)?;
        if !self.present.contains(&idx) {
            return None;
        }
        let v = *self.frames[i].values.get(idx)? as f64;
        v.is_finite().then_some(v)
    }
    fn has_channel(&self, channel: &str) -> bool {
        self.index.get(channel).map_or(false, |i| self.present.contains(i))
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CellStat {
    /// Sum of bilinear weights (≈ effective sample count).
    pub weight: f64,
    sum_ratio: f64,
    /// Raw samples that touched this cell.
    pub hits: u32,
}

impl CellStat {
    pub fn mean_ratio(&self) -> Option<f64> {
        (self.weight > 0.0).then(|| self.sum_ratio / self.weight)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rejections {
    pub low_rpm: usize,
    pub cold: usize,
    pub lambda_out_of_window: usize,
    pub transient: usize,
    pub missing_channels: usize,
    pub off_table: usize,
}

impl Rejections {
    pub fn total(&self) -> usize {
        self.low_rpm + self.cold + self.lambda_out_of_window + self.transient + self.missing_channels + self.off_table
    }
}

#[derive(Clone, Debug)]
pub struct CorrectionResult {
    pub table_id: String,
    pub stats: Vec<Vec<CellStat>>,
    pub used: usize,
    pub rejected: Rejections,
    pub used_trims: bool,
    pub min_weight: f64,
    pub max_change_pct: f64,
}

impl CorrectionResult {
    /// Clamped multiplicative factor for a cell, or `None` without enough data.
    pub fn factor(&self, r: usize, c: usize) -> Option<f64> {
        let s = self.stats.get(r)?.get(c)?;
        if s.weight < self.min_weight {
            return None;
        }
        let lim = self.max_change_pct / 100.0;
        s.mean_ratio().map(|m| m.clamp(1.0 - lim, 1.0 + lim))
    }

    /// Suggested change in percent for a cell (`+4.2` = raise 4.2 %).
    pub fn change_pct(&self, r: usize, c: usize) -> Option<f64> {
        self.factor(r, c).map(|f| (f - 1.0) * 100.0)
    }

    pub fn cells_with_data(&self) -> usize {
        let rows = self.stats.len();
        let cols = self.stats.first().map_or(0, |r| r.len());
        (0..rows)
            .flat_map(|r| (0..cols).map(move |c| (r, c)))
            .filter(|&(r, c)| self.factor(r, c).is_some())
            .count()
    }

    /// Bounding box `(r0, r1, c0, c1)` of cells that have a suggestion.
    pub fn data_bounds(&self) -> Option<(usize, usize, usize, usize)> {
        let mut b: Option<(usize, usize, usize, usize)> = None;
        for r in 0..self.stats.len() {
            for c in 0..self.stats[r].len() {
                if self.factor(r, c).is_some() {
                    b = Some(match b {
                        None => (r, r, c, c),
                        Some((r0, r1, c0, c1)) => (r0.min(r), r1.max(r), c0.min(c), c1.max(c)),
                    });
                }
            }
        }
        b
    }
}

/// Apply suggestions to the cells inside `(r0, r1, c0, c1)` that have data. Returns cells changed.
pub fn apply(values: &mut [Vec<f64>], result: &CorrectionResult, bounds: (usize, usize, usize, usize)) -> usize {
    let (r0, r1, c0, c1) = bounds;
    let mut n = 0;
    for r in r0..=r1.min(values.len().saturating_sub(1)) {
        for c in c0..=c1.min(values[r].len().saturating_sub(1)) {
            if let Some(f) = result.factor(r, c) {
                values[r][c] = (values[r][c] * f * 100.0).round() / 100.0;
                n += 1;
            }
        }
    }
    n
}

/// Index of the last sample at or before `t` (samples are time-ordered).
fn sample_at_or_before(src: &dyn SampleSource, t: f64, hint: usize) -> usize {
    let mut i = hint.min(src.len().saturating_sub(1));
    while i > 0 && src.time(i) > t {
        i -= 1;
    }
    i
}

pub fn analyze(table: &XdfTable, src: &dyn SampleSource, settings: &CorrectionSettings) -> CorrectionResult {
    let rows = table.values.len();
    let cols = table.values.first().map_or(0, |r| r.len());
    let mut stats = vec![vec![CellStat::default(); cols]; rows];
    let mut rejected = Rejections::default();
    let mut used = 0;
    let has_tps = src.has_channel("tps");
    let has_ect = src.has_channel("ect");
    let used_trims = src.has_channel("stft_b1") || src.has_channel("ltft_b1");

    for i in 0..src.len() {
        let Some(lambda) = src.value(i, "lambda") else {
            rejected.missing_channels += 1;
            continue;
        };
        if !(settings.lambda_min..=settings.lambda_max).contains(&lambda) {
            rejected.lambda_out_of_window += 1;
            continue;
        }
        // Operating point that produced this lambda reading (sensor transport delay).
        let j = sample_at_or_before(src, src.time(i) - settings.lambda_delay_s, i);
        let v = |ch: &str| src.value(j, ch);
        let Some(rpm) = v("rpm") else {
            rejected.missing_channels += 1;
            continue;
        };
        if rpm < settings.min_rpm {
            rejected.low_rpm += 1;
            continue;
        }
        if has_ect && v("ect").map_or(false, |e| e < settings.min_ect_c) {
            rejected.cold += 1;
            continue;
        }
        if has_tps && j > 0 {
            // Rate over a short window, not adjacent frames: frame-to-frame differencing turns
            // ordinary TPS noise at 20+ Hz into huge rates and rejects steady cruise.
            let k = sample_at_or_before(src, src.time(j) - TPS_RATE_WINDOW_S, j - 1);
            let dt = src.time(j) - src.time(k);
            if let (Some(a), Some(b)) = (src.value(k, "tps"), v("tps")) {
                // Only judge once at least half a window of history exists (log start).
                if dt >= TPS_RATE_WINDOW_S * 0.5 && ((b - a) / dt).abs() > settings.max_tps_rate_pct_s {
                    rejected.transient += 1;
                    continue;
                }
            }
        }
        let lookup = |ch: &str| v(ch).unwrap_or(f64::NAN) as f32;
        let Some((row, col)) = table_position(table, lookup) else {
            rejected.off_table += 1;
            continue;
        };
        if crate::live_trace::out_of_range(table, lookup).len() > 0 {
            // Clamped to the edge: the error belongs to a cell that doesn't exist.
            rejected.off_table += 1;
            continue;
        }
        let target = match &settings.target {
            TargetLambda::Constant(t) => Some(*t),
            TargetLambda::Table(tt) => table_position(tt, lookup).map(|(r, c)| bilinear(&tt.values, r, c)),
        };
        let Some(target) = target.filter(|t| *t > 0.3) else {
            rejected.missing_channels += 1;
            continue;
        };
        let trim = (1.0 + v("stft_b1").unwrap_or(0.0) / 100.0) * (1.0 + v("ltft_b1").unwrap_or(0.0) / 100.0);
        let ratio = lambda / target * trim;

        // Bilinear spread over the 4 neighbouring cells.
        let r0 = (row.floor() as usize).min(rows.saturating_sub(1));
        let c0 = (col.floor() as usize).min(cols.saturating_sub(1));
        let (tr, tc) = ((row - r0 as f32) as f64, (col - c0 as f32) as f64);
        for (dr, dc, w) in [
            (0, 0, (1.0 - tr) * (1.0 - tc)),
            (0, 1, (1.0 - tr) * tc),
            (1, 0, tr * (1.0 - tc)),
            (1, 1, tr * tc),
        ] {
            let (r, c) = (r0 + dr, c0 + dc);
            if w <= 1e-9 || r >= rows || c >= cols {
                continue;
            }
            let s = &mut stats[r][c];
            s.weight += w;
            s.sum_ratio += ratio * w;
            s.hits += 1;
        }
        used += 1;
    }

    CorrectionResult {
        table_id: table.id.clone(),
        stats,
        used,
        rejected,
        used_trims,
        min_weight: settings.min_weight,
        max_change_pct: settings.max_change_pct,
    }
}

fn bilinear(values: &[Vec<f64>], row: f32, col: f32) -> f64 {
    let rows = values.len();
    let cols = values.first().map_or(0, |r| r.len());
    if rows == 0 || cols == 0 {
        return f64::NAN;
    }
    let r0 = (row.floor().max(0.0) as usize).min(rows - 1);
    let c0 = (col.floor().max(0.0) as usize).min(cols - 1);
    let r1 = (r0 + 1).min(rows - 1);
    let c1 = (c0 + 1).min(cols - 1);
    let (tr, tc) = ((row - r0 as f32) as f64, (col - c0 as f32) as f64);
    let top = values[r0][c0] * (1.0 - tc) + values[r0][c1] * tc;
    let bot = values[r1][c0] * (1.0 - tc) + values[r1][c1] * tc;
    top * (1.0 - tr) + bot * tr
}

#[cfg(test)]
mod tests {
    use super::*;

    /// In-memory log: rows of (time, [(channel, value)]).
    struct VecSource {
        rows: Vec<(f64, HashMap<&'static str, f64>)>,
    }

    impl SampleSource for VecSource {
        fn len(&self) -> usize {
            self.rows.len()
        }
        fn time(&self, i: usize) -> f64 {
            self.rows[i].0
        }
        fn value(&self, i: usize, ch: &str) -> Option<f64> {
            self.rows[i].1.get(ch).copied()
        }
        fn has_channel(&self, ch: &str) -> bool {
            self.rows.iter().any(|(_, m)| m.contains_key(ch))
        }
    }

    fn ve_table() -> XdfTable {
        let mut t = crate::definitions::XdfDocument::sample().tables.remove(0);
        t.x_axis.units = "RPM".into();
        t.x_axis.labels = vec![1000.0, 2000.0, 3000.0];
        t.y_axis.units = "kPa".into();
        t.y_axis.labels = vec![30.0, 60.0, 90.0];
        t.values = vec![vec![50.0; 3]; 3];
        t
    }

    fn sample(t: f64, rpm: f64, map: f64, lambda: f64, extra: &[(&'static str, f64)]) -> (f64, HashMap<&'static str, f64>) {
        let mut m: HashMap<&'static str, f64> = [("rpm", rpm), ("map", map), ("lambda", lambda)].into_iter().collect();
        m.extend(extra.iter().copied());
        (t, m)
    }

    fn settings() -> CorrectionSettings {
        CorrectionSettings { lambda_delay_s: 0.0, min_weight: 3.0, ..Default::default() }
    }

    #[test]
    fn lean_cell_gets_raised_by_lambda_ratio() {
        // Sitting exactly on cell (1,1): measured 1.08 vs target 1.0 => +8 %.
        let rows = (0..10).map(|i| sample(i as f64 * 0.1, 2000.0, 60.0, 1.08, &[])).collect();
        let res = analyze(&ve_table(), &VecSource { rows }, &settings());
        assert_eq!(res.used, 10);
        let pct = res.change_pct(1, 1).unwrap();
        assert!((pct - 8.0).abs() < 1e-6, "{}", pct);
        assert_eq!(res.change_pct(0, 0), None, "no data elsewhere");

        let mut v = ve_table().values;
        let n = apply(&mut v, &res, (0, 2, 0, 2));
        assert_eq!(n, 1);
        assert_eq!(v[1][1], 54.0);
    }

    #[test]
    fn closed_loop_trims_count_as_error() {
        // Lambda pinned at target by the ECU, but +5 % STFT and +3 % LTFT => table is lean.
        let rows = (0..10)
            .map(|i| sample(i as f64 * 0.1, 2000.0, 60.0, 1.0, &[("stft_b1", 5.0), ("ltft_b1", 3.0)]))
            .collect();
        let res = analyze(&ve_table(), &VecSource { rows }, &settings());
        assert!(res.used_trims);
        let pct = res.change_pct(1, 1).unwrap();
        assert!((pct - 8.15).abs() < 1e-6, "{}", pct);
    }

    #[test]
    fn target_table_is_looked_up_per_sample() {
        let mut target = ve_table();
        target.values = vec![vec![0.85; 3]; 3]; // power enrichment everywhere
        let rows = (0..10).map(|i| sample(i as f64 * 0.1, 2000.0, 60.0, 0.85, &[])).collect();
        let s = CorrectionSettings { target: TargetLambda::Table(target), ..settings() };
        let res = analyze(&ve_table(), &VecSource { rows }, &s);
        assert!(res.change_pct(1, 1).unwrap().abs() < 1e-9, "on target => no change");
    }

    #[test]
    fn between_cells_spreads_bilinear_weight() {
        // Halfway between rpm 2000 and 3000 on the 60 kPa row.
        let rows = (0..20).map(|i| sample(i as f64 * 0.1, 2500.0, 60.0, 1.1, &[])).collect();
        let res = analyze(&ve_table(), &VecSource { rows }, &settings());
        assert!((res.stats[1][1].weight - 10.0).abs() < 1e-9);
        assert!((res.stats[1][2].weight - 10.0).abs() < 1e-9);
        assert_eq!(res.stats[0][1].weight, 0.0);
    }

    #[test]
    fn filters_reject_bad_samples() {
        let rows = vec![
            sample(0.0, 400.0, 60.0, 1.0, &[("tps", 10.0)]),                 // idle/crank
            sample(0.1, 2000.0, 60.0, 2.5, &[("tps", 10.0)]),                // fuel cut
            sample(0.2, 2000.0, 60.0, 1.0, &[("ect", 40.0), ("tps", 10.0)]), // cold
            sample(0.3, 2000.0, 150.0, 1.0, &[("tps", 10.0)]),               // off table (MAP > 90)
            sample(0.4, 2000.0, 60.0, 1.0, &[("tps", 10.0)]),
            sample(0.5, 2000.0, 60.0, 1.0, &[("tps", 60.0)]),   // 500 %/s tip-in
        ];
        let res = analyze(&ve_table(), &VecSource { rows }, &settings());
        let r = &res.rejected;
        assert_eq!((r.low_rpm, r.lambda_out_of_window, r.cold, r.off_table, r.transient), (1, 1, 1, 1, 1));
        assert_eq!(res.used, 1);
    }

    #[test]
    fn tps_noise_is_not_a_transient() {
        // Steady cruise with ±2 % TPS jitter at 20 Hz (adjacent-frame rate would be ±80 %/s).
        let rows = (0..40)
            .map(|i| sample(i as f64 * 0.05, 2000.0, 60.0, 1.0, &[("tps", 20.0 + if i % 2 == 0 { 2.0 } else { -2.0 })]))
            .collect();
        let res = analyze(&ve_table(), &VecSource { rows }, &settings());
        assert_eq!(res.rejected.transient, 0);
    }

    #[test]
    fn change_is_clamped_and_needs_min_weight() {
        let rows = (0..2).map(|i| sample(i as f64 * 0.1, 2000.0, 60.0, 1.3, &[])).collect();
        let res = analyze(&ve_table(), &VecSource { rows }, &settings());
        assert_eq!(res.change_pct(1, 1), None, "2 samples < min weight 3");

        let rows = (0..10).map(|i| sample(i as f64 * 0.1, 2000.0, 60.0, 1.3, &[])).collect();
        let res = analyze(&ve_table(), &VecSource { rows }, &settings());
        assert!((res.change_pct(1, 1).unwrap() - 15.0).abs() < 1e-9, "clamped to max_change_pct");
    }

    #[test]
    fn frame_source_ignores_never_logged_channels() {
        use crate::telemetry::TelemetryFrame;
        let index: HashMap<String, usize> =
            [("rpm", 0), ("map", 1), ("lambda", 2), ("ect", 3)].iter().map(|(k, v)| (k.to_string(), *v)).collect();
        // ECT column absent from the CSV => imported as 0.0 in every frame.
        let frames: Vec<TelemetryFrame> = (0..10)
            .map(|i| TelemetryFrame { time_sec: i as f64 * 0.1, unix_ms: 0, values: vec![2000.0, 60.0, 1.05, 0.0] })
            .collect();
        let src = FrameSource::new(&index, &frames);
        assert!(!src.has_channel("ect"));
        let res = analyze(&ve_table(), &src, &settings());
        assert_eq!(res.rejected.cold, 0, "missing ECT must not mean 0 °C");
        assert_eq!(res.used, 10);
    }

    #[test]
    fn lambda_delay_pairs_with_earlier_operating_point() {
        // Engine moved from 2000 rpm to 3000 rpm at t=1.0; lambda read at 1.1 belongs to 2000.
        let mut rows = vec![];
        for i in 0..10 {
            rows.push(sample(i as f64 * 0.1, 2000.0, 60.0, 1.0, &[]));
        }
        rows.push(sample(1.0, 3000.0, 60.0, 1.0, &[]));
        rows.push(sample(1.1, 3000.0, 60.0, 1.2, &[]));
        let s = CorrectionSettings { lambda_delay_s: 0.15, ..settings() };
        let res = analyze(&ve_table(), &VecSource { rows }, &s);
        // Both readings after the step (t=1.0, 1.1) pair back to 2000 rpm operating points.
        assert_eq!(res.stats[1][1].hits, 12, "the lean reading lands on the 2000 rpm cell");
        assert_eq!(res.stats[1][2].hits, 0, "nothing attributed to 3000 rpm yet");
        let mean = res.stats[1][1].mean_ratio().unwrap();
        assert!((mean - (11.0 + 1.2) / 12.0).abs() < 1e-9, "{}", mean);
    }
}
