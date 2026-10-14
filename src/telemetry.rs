use serde::Deserialize;
use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

// ---------------------------------------------------------------------------
// JSON channel definition
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Deserialize)]
pub struct TelemetryChannelFile {
    pub version: u32,
    pub title: Option<String>,
    pub channels: Vec<TelemetryChannelDef>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct TelemetryChannelDef {
    pub id: String,
    pub label: String,
    pub unit: String,
    pub priority: u8,
    pub interval_ms: u64,
    pub request: ChannelRequestDef,
    pub decode: String,
    #[serde(default = "default_true")]
    pub gauge: bool,
    pub plot: Option<PlotStyleDef>,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize)]
pub struct PlotStyleDef {
    pub scale: f64,
    pub color: [u8; 3],
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "service", rename_all = "snake_case")]
pub enum ChannelRequestDef {
    Mode01 { pid: u8 },
    Uds22 { did: u16 },
}

impl TelemetryChannelFile {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
        let file: TelemetryChannelFile =
            serde_json::from_str(&text).map_err(|e| format!("Invalid telemetry JSON: {}", e))?;
        if file.channels.is_empty() {
            return Err("telemetry channel file has no channels".to_string());
        }
        Ok(file)
    }

    pub fn default_path() -> PathBuf {
        PathBuf::from("telemetry-channels.json")
    }

    pub fn load_default() -> Self {
        Self::load(&Self::default_path()).unwrap_or_else(|e| {
            eprintln!("telemetry: {} — using built-in fallback", e);
            Self::builtin_fallback()
        })
    }

    fn builtin_fallback() -> Self {
        serde_json::from_str(
            r#"{
            "version": 1,
            "channels": [
              {"id":"rpm","label":"RPM","unit":"RPM","priority":1,"interval_ms":50,
               "request":{"service":"mode01","pid":12},"decode":"pid_0c_rpm","gauge":true,
               "plot":{"scale":1.0,"color":[0,229,255]}}
            ]
          }"#,
        )
        .expect("builtin telemetry fallback")
    }
}

impl TelemetryChannelDef {
    pub fn build_request_bytes(&self) -> Vec<u8> {
        match &self.request {
            ChannelRequestDef::Mode01 { pid } => vec![0x01, *pid],
            ChannelRequestDef::Uds22 { did } => vec![0x22, (did >> 8) as u8, (did & 0xFF) as u8],
        }
    }

    pub fn plot_scale(&self) -> f64 {
        self.plot.as_ref().map(|p| p.scale).unwrap_or(1.0)
    }

    pub fn plot_color(&self) -> [u8; 3] {
        self.plot
            .as_ref()
            .map(|p| p.color)
            .unwrap_or([180, 180, 180])
    }
}

// ---------------------------------------------------------------------------
// Response decode (definition-driven by decode id)
// ---------------------------------------------------------------------------

pub fn decode_channel_value(decode: &str, resp: &[u8]) -> Option<f32> {
    if resp.len() < 2 {
        return None;
    }
    if resp[0] == 0x7F {
        return None;
    }

    match decode {
        "pid_0c_rpm" => {
            if resp[0] == 0x41 && resp.len() >= 4 && resp[1] == 0x0C {
                let raw = ((resp[2] as u16) << 8) | (resp[3] as u16);
                Some(raw as f32 / 4.0)
            } else {
                None
            }
        }
        "pid_0b_map_kpa" => {
            if resp[0] == 0x41 && resp.len() >= 3 && resp[1] == 0x0B {
                Some(resp[2] as f32)
            } else {
                None
            }
        }
        "pid_04_load" => {
            if resp[0] == 0x41 && resp.len() >= 3 && resp[1] == 0x04 {
                Some((resp[2] as f32) * 100.0 / 255.0)
            } else {
                None
            }
        }
        "pid_05_ect" => {
            if resp[0] == 0x41 && resp.len() >= 3 && resp[1] == 0x05 {
                Some((resp[2] as f32) - 40.0)
            } else {
                None
            }
        }
        "pid_0e_timing" => {
            if resp[0] == 0x41 && resp.len() >= 3 && resp[1] == 0x0E {
                Some((resp[2] as f32) / 2.0 - 64.0)
            } else {
                None
            }
        }
        "pid_0f_iat" => {
            if resp[0] == 0x41 && resp.len() >= 3 && resp[1] == 0x0F {
                Some((resp[2] as f32) - 40.0)
            } else {
                None
            }
        }
        "pid_11_tps" => {
            if resp[0] == 0x41 && resp.len() >= 3 && resp[1] == 0x11 {
                Some((resp[2] as f32) * 100.0 / 255.0)
            } else {
                None
            }
        }
        "pid_24_lambda" => {
            if resp[0] == 0x41 && resp.len() >= 4 && (resp[1] == 0x24 || resp[1] == 0x34) {
                let raw = ((resp[2] as u16) << 8) | (resp[3] as u16);
                Some(raw as f32 / 32768.0)
            } else {
                None
            }
        }
        "did_f010_vbatt" => {
            if resp[0] == 0x62 && resp.len() >= 5 && resp[1] == 0xF0 && resp[2] == 0x10 {
                let raw = ((resp[3] as u16) << 8) | (resp[4] as u16);
                Some(raw as f32 / 1000.0)
            } else {
                None
            }
        }
        "did_f011_oil_temp" => {
            if resp[0] == 0x62 && resp.len() >= 4 && resp[1] == 0xF0 && resp[2] == 0x11 {
                Some((resp[3] as f32) - 40.0)
            } else {
                None
            }
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Scheduler — priority + per-channel sample interval
// ---------------------------------------------------------------------------

pub struct TelemetryScheduler {
    last_poll: Vec<Instant>,
    tick_ms: u64,
}

impl TelemetryScheduler {
    pub fn new(channel_count: usize, tick_ms: u64) -> Self {
        // Stagger start so every channel is eligible on the first scheduler tick.
        let stale = Instant::now() - std::time::Duration::from_secs(3600);
        Self {
            last_poll: vec![stale; channel_count],
            tick_ms,
        }
    }

    /// Channel indices due for a request, highest priority (lowest number) first.
    pub fn due_indices(&self, channels: &[TelemetryChannelDef], now: Instant) -> Vec<usize> {
        let mut due: Vec<(u8, usize)> = channels
            .iter()
            .enumerate()
            .filter_map(|(idx, ch)| {
                let elapsed = now.duration_since(self.last_poll[idx]).as_millis() as u64;
                if elapsed >= ch.interval_ms {
                    Some((ch.priority, idx))
                } else {
                    None
                }
            })
            .collect();
        due.sort_by_key(|(pri, _)| *pri);
        due.into_iter().map(|(_, idx)| idx).collect()
    }

    pub fn mark_polled(&mut self, idx: usize, when: Instant) {
        if idx < self.last_poll.len() {
            self.last_poll[idx] = when;
        }
    }

    pub fn tick_interval(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.tick_ms)
    }
}

// ---------------------------------------------------------------------------
// Dynamic ring buffer + optional recording
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct TelemetryFrame {
    pub time_sec: f64,
    pub values: Vec<f32>,
}

pub struct TelemetryBuffer {
    pub channels: Vec<TelemetryChannelDef>,
    pub id_index: HashMap<String, usize>,
    pub latest: Vec<f32>,
    pub frames: VecDeque<TelemetryFrame>,
    pub max_capacity: usize,
    pub is_recording: bool,
    pub record_frames: Vec<TelemetryFrame>,
    pub start_time: Instant,
    pub scheduler: TelemetryScheduler,
    pub definition_title: String,
}

impl TelemetryBuffer {
    pub fn from_definition(def: &TelemetryChannelFile, capacity: usize) -> Self {
        let channels = def.channels.clone();
        let id_index: HashMap<String, usize> = channels
            .iter()
            .enumerate()
            .map(|(i, c)| (c.id.clone(), i))
            .collect();
        let channel_count = channels.len();
        Self {
            channels,
            id_index,
            latest: vec![0.0; channel_count],
            frames: VecDeque::with_capacity(capacity.min(1024)),
            max_capacity: capacity,
            is_recording: false,
            record_frames: Vec::new(),
            start_time: Instant::now(),
            scheduler: TelemetryScheduler::new(channel_count, 50),
            definition_title: def.title.clone().unwrap_or_else(|| "Live channels".to_string()),
        }
    }

    pub fn channel_idx(&self, id: &str) -> Option<usize> {
        self.id_index.get(id).copied()
    }

    pub fn value(&self, id: &str) -> f32 {
        self.channel_idx(id)
            .and_then(|i| self.latest.get(i).copied())
            .unwrap_or(0.0)
    }

    pub fn set_channel_value(&mut self, idx: usize, value: f32) {
        if idx < self.latest.len() {
            self.latest[idx] = value;
        }
    }

    pub fn push_frame(&mut self) {
        let time_sec = self.start_time.elapsed().as_secs_f64();
        let frame = TelemetryFrame {
            time_sec,
            values: self.latest.clone(),
        };
        if self.frames.len() >= self.max_capacity {
            self.frames.pop_front();
        }
        self.frames.push_back(frame.clone());
        if self.is_recording {
            self.record_frames.push(frame);
        }
    }

    pub fn clear(&mut self) {
        self.frames.clear();
        self.record_frames.clear();
        self.latest.fill(0.0);
        self.start_time = Instant::now();
        let now = Instant::now();
        for t in &mut self.scheduler.last_poll {
            *t = now;
        }
    }

    pub fn set_recording(&mut self, on: bool) {
        if on && !self.is_recording {
            self.record_frames.clear();
        }
        self.is_recording = on;
    }

    pub fn export_csv(&self, path: &str) -> Result<(), String> {
        let export_rows: Vec<&TelemetryFrame> = if !self.record_frames.is_empty() {
            self.record_frames.iter().collect()
        } else {
            self.frames.iter().collect()
        };
        if export_rows.is_empty() {
            return Err("No telemetry samples to export".to_string());
        }

        let mut file = File::create(path).map_err(|e| format!("Failed to create CSV: {}", e))?;
        let header_ids: Vec<&str> = self.channels.iter().map(|c| c.id.as_str()).collect();
        writeln!(file, "Time_s,{}", header_ids.join(","))
            .map_err(|e| format!("Failed to write header: {}", e))?;
        for frame in export_rows {
            let mut line = format!("{:.3}", frame.time_sec);
            for v in &frame.values {
                line.push(',');
                line.push_str(&format!("{:.4}", v));
            }
            writeln!(file, "{}", line).map_err(|e| format!("Failed to write row: {}", e))?;
        }
        Ok(())
    }

    pub fn import_csv(path: &str, def: &TelemetryChannelFile) -> Result<Vec<TelemetryFrame>, String> {
        let file = File::open(path).map_err(|e| format!("Failed to open CSV: {}", e))?;
        let reader = BufReader::new(file);
        let mut lines: Vec<String> = reader
            .lines()
            .map(|l| l.map_err(|e| e.to_string()))
            .collect::<Result<_, _>>()?;
        if lines.is_empty() {
            return Ok(Vec::new());
        }
        let header: Vec<&str> = lines[0].split(',').map(|s| s.trim()).collect();
        if header.len() < 2 || header[0] != "Time_s" {
            return Err("CSV header must start with Time_s".to_string());
        }
        let col_map: HashMap<String, usize> = def
            .channels
            .iter()
            .enumerate()
            .map(|(i, c)| (c.id.clone(), i + 1))
            .collect();
        let mut out = Vec::new();
        for line in lines.iter().skip(1) {
            if line.trim().is_empty() {
                continue;
            }
            let parts: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
            let time_sec = parts[0].parse().unwrap_or(0.0);
            let mut values = vec![0.0f32; def.channels.len()];
            for (col_idx, col_name) in header.iter().enumerate().skip(1) {
                if let Some(&ch_idx) = col_map.get(*col_name) {
                    if col_idx < parts.len() {
                        values[ch_idx - 1] = parts[col_idx].parse().unwrap_or(0.0);
                    }
                }
            }
            out.push(TelemetryFrame { time_sec, values });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_default_channels() {
        let def = TelemetryChannelFile::load_default();
        assert!(def.channels.len() >= 8);
        assert!(def.channels.iter().any(|c| c.id == "rpm"));
    }

    #[test]
    fn test_scheduler_respects_interval_and_priority() {
        let def = TelemetryChannelFile::load_default();
        let sched = TelemetryScheduler::new(def.channels.len(), 50);
        let now = Instant::now();
        let due = sched.due_indices(&def.channels, now);
        assert!(!due.is_empty());
        let rpm_idx = def.channels.iter().position(|c| c.id == "rpm").unwrap();
        assert_eq!(due[0], rpm_idx);
    }

    #[test]
    fn test_decode_mode01_rpm() {
        let resp = vec![0x41, 0x0C, 0x1F, 0x40];
        let v = decode_channel_value("pid_0c_rpm", &resp).unwrap();
        assert!((v - 2000.0).abs() < 0.1);
    }

    #[test]
    fn test_dynamic_buffer_csv_roundtrip() {
        let def = TelemetryChannelFile::load_default();
        let mut buf = TelemetryBuffer::from_definition(&def, 100);
        buf.set_channel_value(0, 1234.0);
        buf.push_frame();
        let path = std::env::temp_dir().join("wideband-telemetry-test.csv");
        buf.export_csv(path.to_str().unwrap()).unwrap();
        let imported = TelemetryBuffer::import_csv(path.to_str().unwrap(), &def).unwrap();
        assert_eq!(imported.len(), 1);
        assert!((imported[0].values[0] - 1234.0).abs() < 0.01);
    }
}
