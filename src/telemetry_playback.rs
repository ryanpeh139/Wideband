use crate::telemetry::{TelemetryChannelDef, TelemetryFrame};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

// ---------------------------------------------------------------------------
// Multi-variable channel formulas (C# LogPanel / ChannelMath parity)
// ---------------------------------------------------------------------------

pub fn eval_channel_formula(expression: &str, variables: &HashMap<String, f64>) -> Result<f64, String> {
    let mut index = 0;
    let value = parse_add(expression, variables, &mut index)?;
    skip_ws(expression, &mut index);
    if index != expression.len() {
        return Err("Unexpected character in channel formula.".to_string());
    }
    Ok(value)
}

fn skip_ws(expression: &str, index: &mut usize) {
    while *index < expression.len() {
        let c = expression.as_bytes()[*index] as char;
        if !c.is_whitespace() {
            break;
        }
        *index += 1;
    }
}

fn parse_add(expression: &str, variables: &HashMap<String, f64>, index: &mut usize) -> Result<f64, String> {
    let mut value = parse_mul(expression, variables, index)?;
    loop {
        skip_ws(expression, index);
        if *index >= expression.len() {
            return Ok(value);
        }
        let op = expression.as_bytes()[*index] as char;
        if op != '+' && op != '-' {
            return Ok(value);
        }
        *index += 1;
        let right = parse_mul(expression, variables, index)?;
        value = if op == '+' { value + right } else { value - right };
    }
}

fn parse_mul(expression: &str, variables: &HashMap<String, f64>, index: &mut usize) -> Result<f64, String> {
    let mut value = parse_unary(expression, variables, index)?;
    loop {
        skip_ws(expression, index);
        if *index >= expression.len() {
            return Ok(value);
        }
        let op = expression.as_bytes()[*index] as char;
        if op != '*' && op != '/' {
            return Ok(value);
        }
        *index += 1;
        let right = parse_unary(expression, variables, index)?;
        value = if op == '*' {
            value * right
        } else if right == 0.0 {
            return Err("Division by zero in channel formula.".to_string());
        } else {
            value / right
        };
    }
}

fn parse_unary(expression: &str, variables: &HashMap<String, f64>, index: &mut usize) -> Result<f64, String> {
    skip_ws(expression, index);
    if *index < expression.len() && expression.as_bytes()[*index] as char == '-' {
        *index += 1;
        return Ok(-parse_primary(expression, variables, index)?);
    }
    parse_primary(expression, variables, index)
}

fn parse_primary(expression: &str, variables: &HashMap<String, f64>, index: &mut usize) -> Result<f64, String> {
    skip_ws(expression, index);
    if *index >= expression.len() {
        return Err("Channel formula ended early.".to_string());
    }
    let ch = expression.as_bytes()[*index] as char;
    if ch == '(' {
        *index += 1;
        let value = parse_add(expression, variables, index)?;
        skip_ws(expression, index);
        if *index >= expression.len() || expression.as_bytes()[*index] as char != ')' {
            return Err("Missing closing parenthesis.".to_string());
        }
        *index += 1;
        return Ok(value);
    }
    let start = *index;
    if ch.is_ascii_digit() || ch == '.' {
        while *index < expression.len() {
            let c = expression.as_bytes()[*index] as char;
            if !c.is_ascii_digit() && c != '.' {
                break;
            }
            *index += 1;
        }
        let token = expression[start..*index]
            .parse::<f64>()
            .map_err(|_| "Invalid number.".to_string())?;
        return Ok(token);
    }
    while *index < expression.len() {
        let c = expression.as_bytes()[*index] as char;
        if !c.is_alphanumeric() && c != '_' {
            break;
        }
        *index += 1;
    }
    if start == *index {
        return Err("Unexpected token in channel formula.".to_string());
    }
    let name = expression[start..*index].to_lowercase();
    variables
        .get(&name)
        .copied()
        .ok_or_else(|| format!("Unknown channel '{}'.", name))
}

// ---------------------------------------------------------------------------
// Markers, math channels, imported logs
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct LogMarker {
    pub time_sec: f64,
    pub label: String,
    pub comment: String,
}

#[derive(Clone, Debug)]
pub struct MathChannelDef {
    pub id: String,
    pub label: String,
    pub formula: String,
    pub unit: String,
    pub color: [u8; 3],
}

#[derive(Clone, Debug, Default)]
pub struct ImportedLog {
    pub name: String,
    pub path: Option<PathBuf>,
    pub frames: Vec<TelemetryFrame>,
    pub markers: Vec<LogMarker>,
    /// Lowercase column name -> per-frame values for columns not in the live definition.
    pub extra_values: HashMap<String, Vec<f64>>,
}

impl ImportedLog {
    pub fn duration_sec(&self) -> f64 {
        self.frames.last().map(|f| f.time_sec).unwrap_or(0.0)
    }

    pub fn frame_variables(
        &self,
        frame_idx: usize,
        channel_defs: &[TelemetryChannelDef],
    ) -> HashMap<String, f64> {
        let mut vars = HashMap::new();
        if frame_idx >= self.frames.len() {
            return vars;
        }
        let frame = &self.frames[frame_idx];
        for (i, ch) in channel_defs.iter().enumerate() {
            if i < frame.values.len() {
                vars.insert(ch.id.to_lowercase(), frame.values[i] as f64);
            }
        }
        for (name, series) in &self.extra_values {
            if let Some(v) = series.get(frame_idx) {
                vars.insert(name.clone(), *v);
            }
        }
        vars
    }

    pub fn eval_math(
        &self,
        math: &MathChannelDef,
        frame_idx: usize,
        channel_defs: &[TelemetryChannelDef],
    ) -> Option<f64> {
        let vars = self.frame_variables(frame_idx, channel_defs);
        eval_channel_formula(&math.formula, &vars).ok()
    }
}

pub fn import_log_csv(path: &Path, channel_defs: &[TelemetryChannelDef]) -> Result<ImportedLog, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
    import_log_csv_text(
        &text,
        path.file_name().map(|s| s.to_string_lossy().to_string()),
        Some(path.to_path_buf()),
        channel_defs,
    )
}

pub fn import_log_csv_text(
    text: &str,
    name: Option<String>,
    path: Option<PathBuf>,
    channel_defs: &[TelemetryChannelDef],
) -> Result<ImportedLog, String> {
    let lines: Vec<&str> = text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        return Err("Log file is empty.".to_string());
    }
    let header: Vec<String> = lines[0]
        .split(',')
        .map(|s| s.trim().to_string())
        .collect();
    let header_lower: Vec<String> = header.iter().map(|h| h.to_lowercase()).collect();

    let time_col = header_lower
        .iter()
        .position(|h| h == "time_s" || h == "time" || h == "timestamp")
        .unwrap_or(0);
    let has_unix = header_lower.get(1) == Some(&"unix_ms".to_string());

    let id_to_def: HashMap<String, usize> = channel_defs
        .iter()
        .enumerate()
        .map(|(i, c)| (c.id.to_lowercase(), i))
        .collect();

    let mut col_map: HashMap<String, usize> = HashMap::new();
    for (i, h) in header_lower.iter().enumerate() {
        col_map.insert(h.clone(), i);
    }

    let mut extra_columns: HashMap<String, usize> = HashMap::new();
    for (i, h) in header_lower.iter().enumerate() {
        if i == time_col || (has_unix && i == 1) {
            continue;
        }
        if !id_to_def.contains_key(h) {
            extra_columns.insert(h.clone(), i);
        }
    }

    let mut frames = Vec::new();
    let mut extra_storage: HashMap<String, Vec<f64>> = HashMap::new();
    for name in extra_columns.keys() {
        extra_storage.insert(name.clone(), Vec::new());
    }

    for line in lines.iter().skip(1) {
        let parts: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
        if parts.is_empty() {
            continue;
        }
        let time_sec: f64 = parts
            .get(time_col)
            .and_then(|s| s.parse().ok())
            .unwrap_or(frames.len() as f64 * 0.05);
        let unix_ms: u64 = if has_unix {
            parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0)
        } else {
            0
        };
        let mut values = vec![0.0f32; channel_defs.len()];
        for (ch_id, ch_idx) in &id_to_def {
            if let Some(&col) = col_map.get(ch_id) {
                if col < parts.len() {
                    values[*ch_idx] = parts[col].parse().unwrap_or(0.0);
                }
            }
        }
        for (ename, col_idx) in &extra_columns {
            let v = if *col_idx < parts.len() {
                parts[*col_idx].parse().unwrap_or(0.0)
            } else {
                0.0
            };
            extra_storage.get_mut(ename).unwrap().push(v);
        }
        frames.push(TelemetryFrame {
            time_sec,
            unix_ms,
            values,
        });
    }

    if frames.is_empty() {
        return Err("No data rows in log.".to_string());
    }

    Ok(ImportedLog {
        name: name.unwrap_or_else(|| "imported.csv".to_string()),
        path,
        frames,
        markers: Vec::new(),
        extra_values: extra_storage,
    })
}

// ---------------------------------------------------------------------------
// Playback & compare UI state
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TelemetryViewMode {
    Live,
    Playback,
    CompareOverlay,
    CompareSideBySide,
}

pub struct TelemetryPlaybackState {
    pub mode: TelemetryViewMode,
    pub primary: Option<ImportedLog>,
    pub compare: Option<ImportedLog>,
    pub scrub_frame: usize,
    pub playing: bool,
    pub play_speed: f32,
    pub last_tick: Instant,
    pub math_channels: Vec<MathChannelDef>,
    pub formula_draft: String,
    pub marker_label_draft: String,
    pub marker_comment_draft: String,
    pub selected_marker: Option<usize>,
    pub status_message: String,
}

impl Default for TelemetryPlaybackState {
    fn default() -> Self {
        Self {
            mode: TelemetryViewMode::Live,
            primary: None,
            compare: None,
            scrub_frame: 0,
            playing: false,
            play_speed: 1.0,
            last_tick: Instant::now(),
            math_channels: vec![MathChannelDef {
                id: "afr_error".into(),
                label: "AFR error %".into(),
                formula: "(lambda - target) / target * 100".into(),
                unit: "%".into(),
                color: [255, 184, 0],
            }],
            formula_draft: "lambda / target".into(),
            marker_label_draft: "Mark".into(),
            marker_comment_draft: String::new(),
            selected_marker: None,
            status_message: String::new(),
        }
    }
}

impl TelemetryPlaybackState {
    pub fn set_primary_log(&mut self, log: ImportedLog) {
        self.primary = Some(log);
        self.scrub_frame = 0;
        self.playing = false;
        self.mode = TelemetryViewMode::Playback;
        self.status_message = format!(
            "Loaded {} ({} frames, {:.1}s).",
            self.primary.as_ref().unwrap().name,
            self.primary.as_ref().unwrap().frames.len(),
            self.primary.as_ref().unwrap().duration_sec()
        );
    }

    pub fn set_compare_log(&mut self, log: ImportedLog) {
        self.compare = Some(log);
        if self.mode == TelemetryViewMode::Live {
            self.mode = TelemetryViewMode::CompareOverlay;
        }
        self.status_message = format!(
            "Compare log: {} ({} frames).",
            self.compare.as_ref().unwrap().name,
            self.compare.as_ref().unwrap().frames.len()
        );
    }

    pub fn add_marker_at_scrub(&mut self) {
        if let Some(log) = self.primary.as_mut() {
            if log.frames.is_empty() {
                return;
            }
            let idx = self.scrub_frame.min(log.frames.len() - 1);
            let time_sec = log.frames[idx].time_sec;
            let label = if self.marker_label_draft.trim().is_empty() {
                format!("M{}", log.markers.len() + 1)
            } else {
                self.marker_label_draft.trim().to_string()
            };
            let comment = self.marker_comment_draft.trim().to_string();
            log.markers.push(LogMarker {
                time_sec,
                label: label.clone(),
                comment,
            });
            self.status_message = format!("Marker '{}' at {:.2}s.", label, time_sec);
        }
    }

    pub fn tick_playback(&mut self, dt_sec: f64) {
        if !self.playing {
            return;
        }
        let Some(log) = self.primary.as_ref() else {
            return;
        };
        if log.frames.len() < 2 {
            return;
        }
        let current_time = log.frames[self.scrub_frame.min(log.frames.len() - 1)].time_sec;
        let target_time = current_time + dt_sec * self.play_speed as f64;
        while self.scrub_frame + 1 < log.frames.len()
            && log.frames[self.scrub_frame + 1].time_sec <= target_time
        {
            self.scrub_frame += 1;
        }
        if self.scrub_frame >= log.frames.len() - 1 {
            self.playing = false;
        }
    }

    pub fn scrub_time_sec(&self) -> f64 {
        self.primary
            .as_ref()
            .and_then(|l| l.frames.get(self.scrub_frame))
            .map(|f| f.time_sec)
            .unwrap_or(0.0)
    }

    pub fn clamp_scrub(&mut self) {
        if let Some(log) = &self.primary {
            let max = log.frames.len().saturating_sub(1);
            if self.scrub_frame > max {
                self.scrub_frame = max;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::TelemetryChannelFile;

    #[test]
    fn formula_lambda_ratio() {
        let mut vars = HashMap::new();
        vars.insert("lambda".into(), 0.85);
        vars.insert("target".into(), 0.90);
        let v = eval_channel_formula("lambda / target", &vars).unwrap();
        assert!((v - 0.944).abs() < 0.01);
    }

    #[test]
    fn import_legacy_csv_columns() {
        let def = TelemetryChannelFile::load_default();
        let csv = "rpm,load,lambda,target\n3000,45,0.92,0.90\n3100,46,0.91,0.90\n";
        let log = import_log_csv_text(csv, Some("bench.csv".into()), None, &def.channels).unwrap();
        assert_eq!(log.frames.len(), 2);
        let rpm_idx = def.channels.iter().position(|c| c.id == "rpm").unwrap();
        assert!((log.frames[0].values[rpm_idx] - 3000.0).abs() < 1.0);
        assert!(log.extra_values.contains_key("target"));
    }

    #[test]
    fn playback_scrub_bounds() {
        let mut state = TelemetryPlaybackState::default();
        let def = TelemetryChannelFile::load_default();
        let csv = "Time_s,Unix_ms,rpm\n0,0,1000\n1,0,2000\n";
        let log = import_log_csv_text(csv, None, None, &def.channels).unwrap();
        state.set_primary_log(log);
        state.scrub_frame = 99;
        state.clamp_scrub();
        assert_eq!(state.scrub_frame, 1);
    }
}
