//! Toyota calibration update assistant (Techstream / TIS workflow).
//!
//! Wideband does not install Toyota calibration updates itself: the update file, the ECU unlock,
//! and the reprogramming sequence belong to Toyota's software. This module covers the steps around
//! that update, all read-only: record the ECU before, check that an update file names the ECU's
//! current calibration ID, hand the cable to Techstream, and confirm the new calibration after.

use crate::protocols::EcuIdentification;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Toyota bulletins require a stable 13.5 V supply during reprogramming.
pub const RECOMMENDED_UPDATE_VOLTS: f32 = 13.5;
/// Below this a battery is not being held up by a charger in power-supply mode.
pub const MIN_UPDATE_VOLTS: f32 = 13.0;
/// Below this the battery itself is low.
pub const LOW_BATTERY_VOLTS: f32 = 12.2;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct UpdateSnapshot {
    pub taken_at: String,
    pub stage: String,
    pub profile_id: String,
    pub device: String,
    pub vin: String,
    pub cal_ids: Vec<String>,
    pub cvns: Vec<String>,
    pub vbatt_volts: Option<f32>,
}

impl UpdateSnapshot {
    pub fn capture(
        stage: &str,
        id: &EcuIdentification,
        profile_id: &str,
        device: &str,
        vbatt_volts: Option<f32>,
    ) -> Self {
        Self {
            taken_at: Utc::now().to_rfc3339(),
            stage: stage.to_string(),
            profile_id: profile_id.to_string(),
            device: device.to_string(),
            vin: id.vin.clone(),
            cal_ids: id.cal_ids.clone(),
            cvns: id.cvns.clone(),
            vbatt_volts,
        }
    }

    /// The snapshot taken before an update, kept beside the program until the next one.
    pub fn default_before_path() -> PathBuf {
        PathBuf::from("wideband-ecu-before-update.json")
    }

    /// Every snapshot, one JSON object per line, so the update history is never overwritten.
    pub fn history_path() -> PathBuf {
        PathBuf::from("wideband-ecu-update-history.jsonl")
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| format!("Failed to write {}: {}", path.display(), e))
    }

    pub fn append_history(&self, path: &Path) -> Result<(), String> {
        use std::io::Write;
        let line = serde_json::to_string(self).map_err(|e| e.to_string())?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| format!("Failed to open {}: {}", path.display(), e))?;
        writeln!(file, "{}", line).map_err(|e| format!("Failed to write {}: {}", path.display(), e))
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
        serde_json::from_str(&text).map_err(|e| format!("Invalid snapshot {}: {}", path.display(), e))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckLevel {
    Pass,
    Warn,
    Fail,
}

#[derive(Clone, Debug)]
pub struct CheckItem {
    pub level: CheckLevel,
    pub label: String,
    pub detail: String,
}

impl CheckItem {
    fn new(level: CheckLevel, label: &str, detail: String) -> Self {
        Self {
            level,
            label: label.to_string(),
            detail,
        }
    }
}

/// Pre-update checks. Advisory: Wideband does not run the update, so nothing here unlocks anything.
pub fn readiness(id: &EcuIdentification, vbatt_volts: Option<f32>, cable_label: &str) -> Vec<CheckItem> {
    let mut items = Vec::new();

    if id.cal_ids.is_empty() {
        items.push(CheckItem::new(
            CheckLevel::Fail,
            "ECU identified",
            "Run Identify ECU first so the current calibration ID is known.".to_string(),
        ));
    } else {
        items.push(CheckItem::new(
            CheckLevel::Pass,
            "ECU identified",
            format!("Current Cal ID: {}", id.cal_ids.join(", ")),
        ));
    }

    items.push(match vbatt_volts {
        None => CheckItem::new(
            CheckLevel::Warn,
            "Battery voltage",
            format!(
                "The cable does not report voltage. Measure at the battery: hold {:.1} V with a charger in power-supply mode.",
                RECOMMENDED_UPDATE_VOLTS
            ),
        ),
        Some(v) if v < LOW_BATTERY_VOLTS => CheckItem::new(
            CheckLevel::Fail,
            "Battery voltage",
            format!("{:.1} V — battery is low. Charge it and connect a power supply before any update.", v),
        ),
        Some(v) if v < MIN_UPDATE_VOLTS => CheckItem::new(
            CheckLevel::Warn,
            "Battery voltage",
            format!(
                "{:.1} V — connect a charger in power-supply mode to hold about {:.1} V during the update.",
                v, RECOMMENDED_UPDATE_VOLTS
            ),
        ),
        Some(v) => CheckItem::new(CheckLevel::Pass, "Battery voltage", format!("{:.1} V", v)),
    });

    let cable = cable_label.to_ascii_lowercase();
    if cable.contains("mini-vci") || cable.contains("mvci") || cable.contains("xhorse") {
        items.push(CheckItem::new(
            CheckLevel::Warn,
            "Cable",
            "Mini-VCI clone cables are fine for diagnostics but are a known risk for reflashing. A dropout mid-update can leave the ECU unbootable; a genuine J2534 interface is safer for the update itself.".to_string(),
        ));
    } else if cable.contains("virtual") {
        items.push(CheckItem::new(
            CheckLevel::Fail,
            "Cable",
            "Virtual ECU is selected. Connect the real cable to check this vehicle.".to_string(),
        ));
    } else {
        items.push(CheckItem::new(CheckLevel::Pass, "Cable", cable_label.to_string()));
    }

    items
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PayloadKind {
    /// Motorola S-record lines follow the header.
    SRecords,
    /// Non-text bytes follow the header (often encrypted); Wideband never flashes these.
    Opaque,
    /// Only text was found.
    None,
}

#[derive(Clone, Debug)]
pub struct UpdateFileCheck {
    pub header: String,
    pub payload: PayloadKind,
    /// The ECU's current Cal IDs that appear in the file header.
    pub matched_cal_ids: Vec<String>,
    pub verdict: CheckLevel,
    pub summary: String,
}

/// Uppercase letters and digits only, so `3471-5000` and `34715000` compare equal.
fn normalize_id(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

fn is_s_record_line(line: &str) -> bool {
    let t = line.trim();
    let b = t.as_bytes();
    b.len() >= 10
        && (b[0] == b'S' || b[0] == b's')
        && b[1].is_ascii_digit()
        && b[2..].iter().all(|c| c.is_ascii_hexdigit())
}

/// The readable text at the top of a `.cuw` file, and what kind of payload follows it.
pub fn update_file_header(data: &[u8]) -> (String, PayloadKind) {
    let mut start = 0;
    while start < data.len() && data[start] == 0 {
        start += 1;
    }
    let body = &data[start..];
    let mut text_end = 0;
    while text_end < body.len() {
        let b = body[text_end];
        let printable = b == b'\n' || b == b'\r' || b == b'\t' || (0x20..0x7F).contains(&b);
        if !printable {
            break;
        }
        text_end += 1;
    }
    let text = String::from_utf8_lossy(&body[..text_end]).to_string();
    let mut header_lines = Vec::new();
    let mut saw_records = false;
    for line in text.lines() {
        if is_s_record_line(line) {
            saw_records = true;
            break;
        }
        header_lines.push(line.trim_end());
    }
    let header = header_lines.join("\n").trim().to_string();
    let payload = if saw_records {
        PayloadKind::SRecords
    } else if text_end < body.len() {
        PayloadKind::Opaque
    } else {
        PayloadKind::None
    };
    (header, payload)
}

/// Checks whether an update file's header names one of the ECU's current calibration IDs.
/// The `.cuw` header layout is not published, so this is a text search, not a format parser.
pub fn check_update_file(data: &[u8], current_cal_ids: &[String]) -> UpdateFileCheck {
    let (header, payload) = update_file_header(data);
    let header_norm = normalize_id(&header);
    let matched: Vec<String> = current_cal_ids
        .iter()
        .filter(|cal| {
            let n = normalize_id(cal);
            n.len() >= 4 && header_norm.contains(&n)
        })
        .cloned()
        .collect();

    let (verdict, summary) = if current_cal_ids.is_empty() {
        (
            CheckLevel::Fail,
            "Identify the ECU first; there is no current Cal ID to compare.".to_string(),
        )
    } else if header.is_empty() {
        (
            CheckLevel::Fail,
            "No readable header was found in this file, so it cannot be matched to your ECU.".to_string(),
        )
    } else if !matched.is_empty() {
        (
            CheckLevel::Pass,
            format!(
                "The file header lists your current Cal ID {}. It appears to be meant for this ECU — confirm against the Toyota bulletin before updating.",
                matched.join(", ")
            ),
        )
    } else {
        (
            CheckLevel::Fail,
            format!(
                "None of your current Cal IDs ({}) appear in this file's header. It may be for a different ECU — do not use it until the bulletin confirms it applies.",
                current_cal_ids.join(", ")
            ),
        )
    };

    UpdateFileCheck {
        header,
        payload,
        matched_cal_ids: matched,
        verdict,
        summary,
    }
}

#[derive(Clone, Debug)]
pub struct UpdateComparison {
    pub verdict: CheckLevel,
    pub summary: String,
    pub changes: Vec<(String, String)>,
}

/// Compares the snapshot taken before an update with the ECU after it.
/// `expected_new` is the new Cal ID from the bulletin; empty means it was not entered.
pub fn compare_snapshots(before: &UpdateSnapshot, after: &UpdateSnapshot, expected_new: &str) -> UpdateComparison {
    if !before.vin.is_empty() && !after.vin.is_empty() && normalize_id(&before.vin) != normalize_id(&after.vin) {
        return UpdateComparison {
            verdict: CheckLevel::Fail,
            summary: format!(
                "VIN changed from {} to {}. These snapshots are from different vehicles or ECUs.",
                before.vin, after.vin
            ),
            changes: Vec::new(),
        };
    }
    if after.cal_ids.is_empty() {
        return UpdateComparison {
            verdict: CheckLevel::Fail,
            summary: "The ECU did not report a Cal ID after the update. Run Identify ECU again with the ignition ON.".to_string(),
            changes: Vec::new(),
        };
    }

    let before_norm: Vec<String> = before.cal_ids.iter().map(|c| normalize_id(c)).collect();
    let after_norm: Vec<String> = after.cal_ids.iter().map(|c| normalize_id(c)).collect();
    let removed: Vec<&String> = before
        .cal_ids
        .iter()
        .zip(before_norm.iter())
        .filter(|(_, n)| !after_norm.contains(n))
        .map(|(c, _)| c)
        .collect();
    let added: Vec<&String> = after
        .cal_ids
        .iter()
        .zip(after_norm.iter())
        .filter(|(_, n)| !before_norm.contains(n))
        .map(|(c, _)| c)
        .collect();
    let mut changes = Vec::new();
    for i in 0..removed.len().max(added.len()) {
        changes.push((
            removed.get(i).map(|s| s.to_string()).unwrap_or_default(),
            added.get(i).map(|s| s.to_string()).unwrap_or_default(),
        ));
    }

    let expected = normalize_id(expected_new);
    if !expected.is_empty() {
        if after_norm.contains(&expected) {
            return UpdateComparison {
                verdict: CheckLevel::Pass,
                summary: format!("Update confirmed: the ECU now reports {}.", expected_new.trim()),
                changes,
            };
        }
        return UpdateComparison {
            verdict: CheckLevel::Fail,
            summary: format!(
                "The ECU reports {} but the expected new Cal ID {} is not among them.",
                after.cal_ids.join(", "),
                expected_new.trim()
            ),
            changes,
        };
    }

    if changes.is_empty() {
        UpdateComparison {
            verdict: CheckLevel::Warn,
            summary: format!(
                "No change: the ECU still reports {}. The update did not apply, or there was nothing to update.",
                after.cal_ids.join(", ")
            ),
            changes,
        }
    } else {
        UpdateComparison {
            verdict: CheckLevel::Pass,
            summary: "Calibration ID changed. Enter the bulletin's new Cal ID to confirm it is the expected one.".to_string(),
            changes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id_with(cal_ids: &[&str]) -> EcuIdentification {
        EcuIdentification {
            vin: "5TFAZ5CN0HX000000".to_string(),
            cal_ids: cal_ids.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    fn snap(cal_ids: &[&str]) -> UpdateSnapshot {
        UpdateSnapshot::capture("before", &id_with(cal_ids), "tacoma-can", "test", Some(13.6))
    }

    #[test]
    fn test_readiness_flags_missing_id_low_battery_and_clone_cable() {
        let items = readiness(&id_with(&[]), Some(11.9), "[Mini-VCI J2534] XHorse - MVCI");
        assert_eq!(items[0].level, CheckLevel::Fail);
        assert_eq!(items[1].level, CheckLevel::Fail);
        assert_eq!(items[2].level, CheckLevel::Warn);

        let items = readiness(&id_with(&["34715000"]), Some(13.6), "[J2534 Pass-Thru] Genuine");
        assert!(items.iter().all(|i| i.level == CheckLevel::Pass));

        let items = readiness(&id_with(&["34715000"]), None, "Genuine");
        assert_eq!(items[1].level, CheckLevel::Warn);
    }

    #[test]
    fn test_update_file_matches_current_cal_id_in_header() {
        let file = b"CALIBRATION UPDATE\r\nOLD CAL ID: 3471-5000\r\nNEW CAL ID: 34715100\r\nS10500001234B1\r\n";
        let check = check_update_file(file, &["34715000".to_string(), "A0123000".to_string()]);
        assert_eq!(check.payload, PayloadKind::SRecords);
        assert_eq!(check.matched_cal_ids, vec!["34715000".to_string()]);
        assert_eq!(check.verdict, CheckLevel::Pass);
        assert!(!check.header.contains("S105"));
    }

    #[test]
    fn test_update_file_opaque_payload_and_no_match() {
        let mut file = b"HEADER 99999999\n".to_vec();
        file.extend_from_slice(&[0x00, 0x8F, 0xC3, 0x11, 0xFE]);
        let check = check_update_file(&file, &["34715000".to_string()]);
        assert_eq!(check.payload, PayloadKind::Opaque);
        assert!(check.matched_cal_ids.is_empty());
        assert_eq!(check.verdict, CheckLevel::Fail);
        assert_eq!(check.header, "HEADER 99999999");

        let check = check_update_file(b"HEADER", &[]);
        assert_eq!(check.verdict, CheckLevel::Fail);
    }

    #[test]
    fn test_compare_confirms_expected_new_cal_id() {
        let before = snap(&["34715000", "A0123000"]);
        let after = snap(&["34715100", "A0123000"]);
        let result = compare_snapshots(&before, &after, "3471-5100");
        assert_eq!(result.verdict, CheckLevel::Pass);
        assert_eq!(result.changes, vec![("34715000".to_string(), "34715100".to_string())]);

        let wrong = compare_snapshots(&before, &after, "34715200");
        assert_eq!(wrong.verdict, CheckLevel::Fail);
    }

    #[test]
    fn test_compare_reports_no_change_and_vin_mismatch() {
        let before = snap(&["34715000"]);
        let result = compare_snapshots(&before, &before.clone(), "");
        assert_eq!(result.verdict, CheckLevel::Warn);
        assert!(result.changes.is_empty());

        let mut other = snap(&["34715100"]);
        other.vin = "5TFAZ5CN0HX999999".to_string();
        assert_eq!(compare_snapshots(&before, &other, "").verdict, CheckLevel::Fail);
    }

    #[test]
    fn test_snapshot_round_trip() {
        let dir = std::env::temp_dir().join(format!("wideband-update-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("before.json");
        let before = snap(&["34715000"]);
        before.save(&path).unwrap();
        assert_eq!(UpdateSnapshot::load(&path).unwrap(), before);
        let history = dir.join("history.jsonl");
        before.append_history(&history).unwrap();
        before.append_history(&history).unwrap();
        assert_eq!(std::fs::read_to_string(&history).unwrap().lines().count(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
