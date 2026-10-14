//! Live operating-point trace: where the engine is on the selected table right now.
//!
//! Kept separate from the user's selected (edit) cell — the live cursor must never decide which
//! cell a math operation writes to. Positions are fractional `(row, col)` indices interpolated
//! between axis breakpoints, the same way the ECU blends neighbouring cells.

use crate::definitions::XdfTable;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// How long a trail point stays visible on the 3D surface.
pub const TRAIL_WINDOW: Duration = Duration::from_secs(4);
const MAX_TRAIL_POINTS: usize = 240;

/// Telemetry channel id that feeds an axis, chosen from the axis units. `None` when the units
/// don't identify a channel we log (better no trace than a trace on the wrong axis).
pub fn channel_for_axis_units(units: &str) -> Option<&'static str> {
    let u = units.to_ascii_lowercase();
    if u.contains("rpm") {
        Some("rpm")
    } else if u.contains("kpa") || u.contains("map") || u.contains("boost") {
        Some("map")
    } else if u.contains("g/s") || u.contains("maf") || u.contains("airflow") {
        Some("maf")
    } else if u.contains("load") {
        Some("load")
    } else if u.contains("tps") || u.contains("throttle") {
        Some("tps")
    } else {
        None
    }
}

/// Fractional index of `value` along monotonic `labels` (ascending or descending), clamped to
/// the table edges like an ECU lookup. `None` for an empty or non-monotonic axis.
pub fn axis_position(labels: &[f64], value: f64) -> Option<f32> {
    let n = labels.len();
    if n == 0 || !value.is_finite() {
        return None;
    }
    if n == 1 {
        return Some(0.0);
    }
    let ascending = labels[n - 1] >= labels[0];
    let monotonic = labels
        .windows(2)
        .all(|w| if ascending { w[1] >= w[0] } else { w[1] <= w[0] });
    if !monotonic {
        return None;
    }
    let key = |v: f64| if ascending { v } else { -v };
    let v = key(value);
    if v <= key(labels[0]) {
        return Some(0.0);
    }
    if v >= key(labels[n - 1]) {
        return Some((n - 1) as f32);
    }
    for i in 0..n - 1 {
        let (a, b) = (key(labels[i]), key(labels[i + 1]));
        if v >= a && v <= b {
            let t = if b > a { (v - a) / (b - a) } else { 0.0 };
            return Some(i as f32 + t as f32);
        }
    }
    None
}

/// Live `(row, col)` on `table`, reading channels through `value_of`. `None` when either axis
/// has no matching channel or its breakpoints can't be interpolated.
pub fn table_position(table: &XdfTable, value_of: impl Fn(&str) -> f32) -> Option<(f32, f32)> {
    let x_ch = channel_for_axis_units(&table.x_axis.units)?;
    let y_ch = channel_for_axis_units(&table.y_axis.units)?;
    let col = axis_position(&table.x_axis.labels, value_of(x_ch) as f64)?;
    let row = axis_position(&table.y_axis.labels, value_of(y_ch) as f64)?;
    Some((row, col))
}

/// Nearest whole cell to a fractional position (for the table-grid glow).
pub fn nearest_cell(pos: (f32, f32)) -> (usize, usize) {
    (pos.0.round().max(0.0) as usize, pos.1.round().max(0.0) as usize)
}

/// Recent live positions on the selected table, newest last.
#[derive(Default)]
pub struct LiveTrace {
    points: VecDeque<(Instant, (f32, f32))>,
}

impl LiveTrace {
    pub fn push(&mut self, pos: (f32, f32), now: Instant) {
        self.points.push_back((now, pos));
        while self.points.len() > MAX_TRAIL_POINTS {
            self.points.pop_front();
        }
        self.prune(now);
    }

    pub fn prune(&mut self, now: Instant) {
        while let Some((t, _)) = self.points.front() {
            if now.duration_since(*t) > TRAIL_WINDOW {
                self.points.pop_front();
            } else {
                break;
            }
        }
    }

    pub fn clear(&mut self) {
        self.points.clear();
    }

    pub fn current(&self) -> Option<(f32, f32)> {
        self.points.back().map(|(_, p)| *p)
    }

    /// `(position, age 0..1)` oldest first; age 1.0 = about to expire.
    pub fn trail(&self, now: Instant) -> Vec<((f32, f32), f32)> {
        let window = TRAIL_WINDOW.as_secs_f32();
        self.points
            .iter()
            .map(|(t, p)| (*p, (now.duration_since(*t).as_secs_f32() / window).min(1.0)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_from_units() {
        assert_eq!(channel_for_axis_units("RPM"), Some("rpm"));
        assert_eq!(channel_for_axis_units("MAP (kPa)"), Some("map"));
        assert_eq!(channel_for_axis_units("Load %"), Some("load"));
        assert_eq!(channel_for_axis_units("%"), None, "bare % is ambiguous (tps vs load)");
        assert_eq!(channel_for_axis_units("deg C"), None);
    }

    #[test]
    fn interpolates_between_breakpoints() {
        let rpm = [500.0, 1000.0, 2000.0, 4000.0];
        assert_eq!(axis_position(&rpm, 1500.0), Some(1.5));
        assert_eq!(axis_position(&rpm, 3000.0), Some(2.5));
        assert_eq!(axis_position(&rpm, 100.0), Some(0.0), "clamped low");
        assert_eq!(axis_position(&rpm, 9000.0), Some(3.0), "clamped high");
    }

    #[test]
    fn descending_axis_supported() {
        let kpa = [100.0, 80.0, 60.0, 40.0];
        assert_eq!(axis_position(&kpa, 70.0), Some(1.5));
    }

    #[test]
    fn non_monotonic_axis_has_no_position() {
        assert_eq!(axis_position(&[100.0, 50.0, 200.0], 120.0), None);
    }

    #[test]
    fn trail_expires() {
        let mut t = LiveTrace::default();
        let start = Instant::now();
        t.push((1.0, 1.0), start);
        t.push((2.0, 2.0), start + Duration::from_secs(3));
        t.prune(start + Duration::from_secs(5));
        assert_eq!(t.trail(start + Duration::from_secs(5)).len(), 1);
        assert_eq!(t.current(), Some((2.0, 2.0)));
    }
}
