//! Scanner dashboard gauges (HP Tuners VCM Scanner style): dial / bar / digital, per-gauge
//! range and warning thresholds, saved as `dashboard.json` beside the exe.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GaugeStyle {
    Dial,
    Bar,
    Digital,
}

impl GaugeStyle {
    pub const ALL: [GaugeStyle; 3] = [GaugeStyle::Dial, GaugeStyle::Bar, GaugeStyle::Digital];

    pub fn label(self) -> &'static str {
        match self {
            GaugeStyle::Dial => "Dial",
            GaugeStyle::Bar => "Bar",
            GaugeStyle::Digital => "Digital",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GaugeSpec {
    pub channel: String,
    pub style: GaugeStyle,
    pub min: f64,
    pub max: f64,
    #[serde(default)]
    pub warn_high: Option<f64>,
    #[serde(default)]
    pub warn_low: Option<f64>,
}

impl GaugeSpec {
    /// Sensible defaults for the channels in `telemetry-channels.json`.
    pub fn default_for(channel: &str) -> Self {
        let (style, min, max, lo, hi) = match channel {
            "rpm" => (GaugeStyle::Dial, 0.0, 7000.0, None, Some(6000.0)),
            "map" => (GaugeStyle::Dial, 0.0, 200.0, None, None),
            "tps" => (GaugeStyle::Bar, 0.0, 100.0, None, None),
            "load" => (GaugeStyle::Bar, 0.0, 100.0, None, None),
            "lambda" => (GaugeStyle::Dial, 0.6, 1.4, Some(0.72), Some(1.15)),
            "timing" => (GaugeStyle::Dial, -10.0, 50.0, None, None),
            "maf" => (GaugeStyle::Bar, 0.0, 300.0, None, None),
            "stft_b1" | "ltft_b1" => (GaugeStyle::Bar, -25.0, 25.0, Some(-15.0), Some(15.0)),
            "ect" | "oil_temp" => (GaugeStyle::Dial, -40.0, 130.0, None, Some(110.0)),
            "iat" => (GaugeStyle::Dial, -40.0, 100.0, None, Some(60.0)),
            "veh_speed" => (GaugeStyle::Digital, 0.0, 200.0, None, None),
            "vbatt" | "ecu_v" => (GaugeStyle::Digital, 10.0, 16.0, Some(11.8), Some(15.2)),
            _ => (GaugeStyle::Digital, 0.0, 100.0, None, None),
        };
        Self { channel: channel.to_string(), style, min, max, warn_high: hi, warn_low: lo }
    }

    /// Needle / bar position 0..=1 (clamped; degenerate range = 0).
    pub fn fraction(&self, v: f64) -> f32 {
        if !(self.max > self.min) || !v.is_finite() {
            return 0.0;
        }
        (((v - self.min) / (self.max - self.min)).clamp(0.0, 1.0)) as f32
    }

    pub fn alarm(&self, v: f64) -> bool {
        self.warn_high.map_or(false, |h| v >= h) || self.warn_low.map_or(false, |l| v <= l)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Dashboard {
    pub gauges: Vec<GaugeSpec>,
}

impl Dashboard {
    /// The classic tuning set: RPM, MAP, lambda, timing, TPS, trims, ECT, IAT, battery.
    pub fn default_layout(available: &[String]) -> Self {
        let wanted = ["rpm", "map", "lambda", "timing", "tps", "stft_b1", "ltft_b1", "ect", "iat", "vbatt"];
        Self {
            gauges: wanted
                .iter()
                .filter(|c| available.iter().any(|a| a == *c))
                .map(|c| GaugeSpec::default_for(c))
                .collect(),
        }
    }

    pub fn default_path() -> PathBuf {
        crate::pack_paths::beside_exe_or_cwd("dashboard.json")
    }

    /// Where to save: beside the exe when we can tell, so the release folder stays self-contained.
    pub fn save_path() -> PathBuf {
        crate::pack_paths::exe_parent()
            .map(|d| d.join("dashboard.json"))
            .unwrap_or_else(|| PathBuf::from("dashboard.json"))
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        serde_json::from_str(&text).map_err(|e| format!("dashboard.json: {}", e))
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fraction_clamps_and_handles_bad_ranges() {
        let g = GaugeSpec::default_for("rpm");
        assert_eq!(g.fraction(3500.0), 0.5);
        assert_eq!(g.fraction(-100.0), 0.0);
        assert_eq!(g.fraction(9000.0), 1.0);
        let bad = GaugeSpec { min: 5.0, max: 5.0, ..g };
        assert_eq!(bad.fraction(5.0), 0.0);
    }

    #[test]
    fn alarms_on_both_thresholds() {
        let lam = GaugeSpec::default_for("lambda");
        assert!(lam.alarm(1.2) && lam.alarm(0.7) && !lam.alarm(1.0));
        assert!(!GaugeSpec::default_for("map").alarm(250.0), "no thresholds = never alarms");
    }

    #[test]
    fn default_layout_only_uses_logged_channels_and_roundtrips() {
        let avail = vec!["rpm".to_string(), "lambda".to_string(), "boost_custom".to_string()];
        let d = Dashboard::default_layout(&avail);
        assert_eq!(d.gauges.iter().map(|g| g.channel.as_str()).collect::<Vec<_>>(), vec!["rpm", "lambda"]);
        let path = std::env::temp_dir().join("wideband-dashboard-test.json");
        d.save(&path).unwrap();
        assert_eq!(Dashboard::load(&path).unwrap(), d);
    }
}
