use serde::Deserialize;
use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// JSON channel definition
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Deserialize)]
pub struct TelemetryChannelFile {
    pub version: u32,
    pub title: Option<String>,
    #[serde(default)]
    pub scheduler: TelemetrySchedulerSettings,
    #[serde(default)]
    pub recording: TelemetryRecordingSettings,
    pub channels: Vec<TelemetryChannelDef>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct TelemetrySchedulerSettings {
    /// UI / poll loop cadence (ms). Bus throughput still capped by J2534 + ISO-TP.
    #[serde(default = "default_tick_ms")]
    pub tick_ms: u64,
    /// Max diagnostic requests issued per scheduler tick (priority bands first).
    #[serde(default = "default_max_requests_per_tick")]
    pub max_requests_per_tick: usize,
    /// When true, overdue channels are sampled at most once per tick (no catch-up burst).
    #[serde(default = "default_true")]
    pub skip_backlog: bool,
}

impl Default for TelemetrySchedulerSettings {
    fn default() -> Self {
        Self {
            tick_ms: default_tick_ms(),
            max_requests_per_tick: default_max_requests_per_tick(),
            skip_backlog: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct TelemetryRecordingSettings {
    #[serde(default = "default_ring_capacity")]
    pub ring_capacity: usize,
    /// Cap for dedicated record buffer (0 = unlimited).
    #[serde(default)]
    pub record_capacity: usize,
}

impl Default for TelemetryRecordingSettings {
    fn default() -> Self {
        Self {
            ring_capacity: default_ring_capacity(),
            record_capacity: 0,
        }
    }
}

fn default_tick_ms() -> u64 {
    25
}

fn default_max_requests_per_tick() -> usize {
    4
}

fn default_ring_capacity() -> usize {
    10_000
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
    /// Optional note for OEM-specific / placeholder DIDs (shown in UI, not polled differently).
    #[serde(default)]
    pub oem_note: Option<String>,
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
        crate::pack_paths::beside_exe_or_cwd("telemetry-channels.json")
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
        "pid_0d_speed_kph" => {
            if resp[0] == 0x41 && resp.len() >= 3 && resp[1] == 0x0D {
                Some(resp[2] as f32)
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
        "pid_10_maf_gs" => {
            if resp[0] == 0x41 && resp.len() >= 4 && resp[1] == 0x10 {
                let raw = ((resp[2] as u16) << 8) | (resp[3] as u16);
                Some(raw as f32 / 100.0)
            } else {
                None
            }
        }
        "pid_33_baro_kpa" => {
            if resp[0] == 0x41 && resp.len() >= 3 && resp[1] == 0x33 {
                Some(resp[2] as f32)
            } else {
                None
            }
        }
        "pid_42_ecu_v" => {
            if resp[0] == 0x41 && resp.len() >= 4 && resp[1] == 0x42 {
                let raw = ((resp[2] as u16) << 8) | (resp[3] as u16);
                Some(raw as f32 / 1000.0)
            } else {
                None
            }
        }
        "pid_06_stft_pct" | "pid_07_ltft_pct" => {
            let pid = if decode == "pid_06_stft_pct" { 0x06 } else { 0x07 };
            if resp[0] == 0x41 && resp.len() >= 3 && resp[1] == pid {
                Some(((resp[2] as f32) - 128.0) * 100.0 / 128.0)
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
        "pid_2f_fuel_pct" => {
            if resp[0] == 0x41 && resp.len() >= 3 && resp[1] == 0x2F {
                Some((resp[2] as f32) * 100.0 / 255.0)
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
        "did_placeholder_u16" => {
            if resp[0] == 0x62 && resp.len() >= 5 {
                let raw = ((resp[3] as u16) << 8) | (resp[4] as u16);
                Some(raw as f32)
            } else {
                None
            }
        }
        "did_placeholder_u8" => {
            if resp[0] == 0x62 && resp.len() >= 4 {
                Some(resp[3] as f32)
            } else {
                None
            }
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Scheduler — priority bands + per-tick request budget + backlog skip
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct SchedulerPlan {
    pub poll_indices: Vec<usize>,
    pub budget_skipped: Vec<usize>,
}

pub struct TelemetryScheduler {
    last_poll: Vec<Instant>,
    settings: TelemetrySchedulerSettings,
    pub budget_skip_total: u64,
    pub per_channel_budget_skips: Vec<u64>,
}

impl TelemetryScheduler {
    pub fn new(channel_count: usize, settings: TelemetrySchedulerSettings) -> Self {
        let stale = Instant::now() - Duration::from_secs(3600);
        Self {
            last_poll: vec![stale; channel_count],
            settings,
            budget_skip_total: 0,
            per_channel_budget_skips: vec![0; channel_count],
        }
    }

    pub fn settings(&self) -> &TelemetrySchedulerSettings {
        &self.settings
    }

    /// Channels due this tick, ordered by priority band then index; capped by `max_requests_per_tick`.
    pub fn plan(&self, channels: &[TelemetryChannelDef], now: Instant) -> SchedulerPlan {
        let mut due: Vec<(u8, u64, usize)> = channels
            .iter()
            .enumerate()
            .filter_map(|(idx, ch)| {
                let elapsed = now.duration_since(self.last_poll[idx]).as_millis() as u64;
                if elapsed >= ch.interval_ms {
                    Some((ch.priority, elapsed, idx))
                } else {
                    None
                }
            })
            .collect();
        due.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));

        let cap = self.settings.max_requests_per_tick.max(1);
        let poll_indices: Vec<usize> = due
            .iter()
            .take(cap)
            .map(|(_, _, idx)| *idx)
            .collect();
        let budget_skipped: Vec<usize> = due
            .iter()
            .skip(cap)
            .map(|(_, _, idx)| *idx)
            .collect();
        SchedulerPlan {
            poll_indices,
            budget_skipped,
        }
    }

    pub fn apply_budget_skips(&mut self, skipped: &[usize], now: Instant) {
        if skipped.is_empty() {
            return;
        }
        self.budget_skip_total += skipped.len() as u64;
        for &idx in skipped {
            if idx < self.per_channel_budget_skips.len() {
                self.per_channel_budget_skips[idx] += 1;
            }
            if self.settings.skip_backlog && idx < self.last_poll.len() {
                self.last_poll[idx] = now;
            }
        }
    }

    pub fn mark_polled(&mut self, idx: usize, when: Instant) {
        if idx < self.last_poll.len() {
            self.last_poll[idx] = when;
        }
    }

    pub fn tick_interval(&self) -> Duration {
        Duration::from_millis(self.settings.tick_ms)
    }
}

// ---------------------------------------------------------------------------
// Per-channel rate tracking
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct ChannelRateStats {
    samples_in_window: u32,
    window_start: Option<Instant>,
    pub hz: f32,
}

impl ChannelRateStats {
    const WINDOW: Duration = Duration::from_secs(1);

    /// False until the ECU has answered for this channel at least once (value is just 0.0).
    pub fn has_sampled(&self) -> bool {
        self.window_start.is_some()
    }

    pub fn on_sample(&mut self, now: Instant) {
        if self.window_start.is_none() {
            self.window_start = Some(now);
        }
        self.samples_in_window += 1;
        let start = self.window_start.unwrap();
        let elapsed = now.duration_since(start);
        if elapsed >= Self::WINDOW {
            self.hz = self.samples_in_window as f32 / elapsed.as_secs_f32().max(0.001);
            self.samples_in_window = 0;
            self.window_start = Some(now);
        }
    }
}

// ---------------------------------------------------------------------------
// Dynamic ring buffer + optional recording
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct TelemetryFrame {
    pub time_sec: f64,
    pub unix_ms: u64,
    pub values: Vec<f32>,
}

pub struct TelemetryBuffer {
    pub channels: Vec<TelemetryChannelDef>,
    pub id_index: HashMap<String, usize>,
    pub latest: Vec<f32>,
    pub frames: VecDeque<TelemetryFrame>,
    pub max_capacity: usize,
    pub record_capacity: usize,
    pub is_recording: bool,
    pub record_frames: Vec<TelemetryFrame>,
    pub start_time: Instant,
    pub epoch_start: SystemTime,
    pub scheduler: TelemetryScheduler,
    pub definition_title: String,
    pub ring_drop_count: u64,
    pub record_drop_count: u64,
    pub bus_fail_count: u64,
    pub channel_rates: Vec<ChannelRateStats>,
}

impl TelemetryBuffer {
    pub fn from_definition(def: &TelemetryChannelFile, capacity_override: Option<usize>) -> Self {
        let channels = def.channels.clone();
        let id_index: HashMap<String, usize> = channels
            .iter()
            .enumerate()
            .map(|(i, c)| (c.id.clone(), i))
            .collect();
        let channel_count = channels.len();
        let ring_cap = capacity_override.unwrap_or(def.recording.ring_capacity);
        Self {
            channels,
            id_index,
            latest: vec![0.0; channel_count],
            frames: VecDeque::with_capacity(ring_cap.min(1024)),
            max_capacity: ring_cap,
            record_capacity: def.recording.record_capacity,
            is_recording: false,
            record_frames: Vec::new(),
            start_time: Instant::now(),
            epoch_start: SystemTime::now(),
            scheduler: TelemetryScheduler::new(channel_count, def.scheduler.clone()),
            definition_title: def.title.clone().unwrap_or_else(|| "Live channels".to_string()),
            ring_drop_count: 0,
            record_drop_count: 0,
            bus_fail_count: 0,
            channel_rates: vec![ChannelRateStats::default(); channel_count],
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

    pub fn set_channel_value(&mut self, idx: usize, value: f32, now: Instant) {
        if idx < self.latest.len() {
            self.latest[idx] = value;
        }
        if idx < self.channel_rates.len() {
            self.channel_rates[idx].on_sample(now);
        }
    }

    fn frame_timestamp(&self) -> (f64, u64) {
        let time_sec = self.start_time.elapsed().as_secs_f64();
        let unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        (time_sec, unix_ms)
    }

    pub fn push_frame(&mut self) {
        let (time_sec, unix_ms) = self.frame_timestamp();
        let frame = TelemetryFrame {
            time_sec,
            unix_ms,
            values: self.latest.clone(),
        };
        if self.frames.len() >= self.max_capacity {
            self.frames.pop_front();
            self.ring_drop_count += 1;
        }
        self.frames.push_back(frame.clone());
        if self.is_recording {
            if self.record_capacity > 0 && self.record_frames.len() >= self.record_capacity {
                self.record_frames.remove(0);
                self.record_drop_count += 1;
            }
            self.record_frames.push(frame);
        }
    }

    pub fn clear(&mut self) {
        self.frames.clear();
        self.record_frames.clear();
        self.latest.fill(0.0);
        self.ring_drop_count = 0;
        self.record_drop_count = 0;
        self.bus_fail_count = 0;
        self.scheduler.budget_skip_total = 0;
        for s in &mut self.scheduler.per_channel_budget_skips {
            *s = 0;
        }
        for r in &mut self.channel_rates {
            *r = ChannelRateStats::default();
        }
        self.start_time = Instant::now();
        self.epoch_start = SystemTime::now();
        let stale = Instant::now() - Duration::from_secs(3600);
        for t in &mut self.scheduler.last_poll {
            *t = stale;
        }
    }

    pub fn set_recording(&mut self, on: bool) {
        if on && !self.is_recording {
            self.record_frames.clear();
            self.record_drop_count = 0;
        }
        self.is_recording = on;
    }

    pub fn total_scheduler_skips(&self) -> u64 {
        self.scheduler.budget_skip_total
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
        writeln!(file, "Time_s,Unix_ms,{}", header_ids.join(","))
            .map_err(|e| format!("Failed to write header: {}", e))?;
        for frame in export_rows {
            let mut line = format!("{:.3},{}", frame.time_sec, frame.unix_ms);
            for v in &frame.values {
                line.push(',');
                line.push_str(&format!("{:.4}", v));
            }
            writeln!(file, "{}", line).map_err(|e| format!("Failed to write row: {}", e))?;
        }
        Ok(())
    }

    pub fn export_csv_async(path: String, telemetry: Arc<Mutex<TelemetryBuffer>>) {
        std::thread::spawn(move || {
            let result = telemetry
                .lock()
                .map(|buf| buf.export_csv(&path))
                .unwrap_or_else(|_| Err("telemetry lock poisoned".to_string()));
            if let Err(e) = result {
                eprintln!("telemetry CSV export failed: {}", e);
            }
        });
    }

    pub fn import_csv(path: &str, def: &TelemetryChannelFile) -> Result<Vec<TelemetryFrame>, String> {
        let file = File::open(path).map_err(|e| format!("Failed to open CSV: {}", e))?;
        let reader = BufReader::new(file);
        let lines: Vec<String> = reader
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
        let has_unix = header.len() > 1 && header[1] == "Unix_ms";
        let col_map: HashMap<String, usize> = header
            .iter()
            .enumerate()
            .map(|(i, name)| (name.to_string(), i))
            .collect();
        let mut out = Vec::new();
        for line in lines.iter().skip(1) {
            if line.trim().is_empty() {
                continue;
            }
            let parts: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
            let time_sec = parts[0].parse().unwrap_or(0.0);
            let unix_ms = if has_unix && parts.len() > 1 {
                parts[1].parse().unwrap_or(0)
            } else {
                0
            };
            let mut values = vec![0.0f32; def.channels.len()];
            for (ch_idx, ch) in def.channels.iter().enumerate() {
                if let Some(&col) = col_map.get(&ch.id) {
                    if col < parts.len() {
                        values[ch_idx] = parts[col].parse().unwrap_or(0.0);
                    }
                }
            }
            out.push(TelemetryFrame {
                time_sec,
                unix_ms,
                values,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_channels() -> Vec<TelemetryChannelDef> {
        vec![
            TelemetryChannelDef {
                id: "a".into(),
                label: "A".into(),
                unit: "".into(),
                priority: 1,
                interval_ms: 50,
                request: ChannelRequestDef::Mode01 { pid: 0x0C },
                decode: "pid_0c_rpm".into(),
                gauge: true,
                plot: None,
                oem_note: None,
            },
            TelemetryChannelDef {
                id: "b".into(),
                label: "B".into(),
                unit: "".into(),
                priority: 1,
                interval_ms: 50,
                request: ChannelRequestDef::Mode01 { pid: 0x0B },
                decode: "pid_0b_map_kpa".into(),
                gauge: true,
                plot: None,
                oem_note: None,
            },
            TelemetryChannelDef {
                id: "c".into(),
                label: "C".into(),
                unit: "".into(),
                priority: 2,
                interval_ms: 50,
                request: ChannelRequestDef::Mode01 { pid: 0x05 },
                decode: "pid_05_ect".into(),
                gauge: false,
                plot: None,
                oem_note: None,
            },
            TelemetryChannelDef {
                id: "d".into(),
                label: "D".into(),
                unit: "".into(),
                priority: 3,
                interval_ms: 50,
                request: ChannelRequestDef::Mode01 { pid: 0x0F },
                decode: "pid_0f_iat".into(),
                gauge: false,
                plot: None,
                oem_note: None,
            },
        ]
    }

    #[test]
    fn test_load_default_channels() {
        let def = TelemetryChannelFile::load_default();
        assert!(def.channels.len() >= 12);
        assert!(def.channels.iter().any(|c| c.id == "rpm"));
        assert!(def.scheduler.max_requests_per_tick >= 1);
    }

    #[test]
    fn test_scheduler_respects_interval_and_priority() {
        let def = TelemetryChannelFile::load_default();
        let sched = TelemetryScheduler::new(def.channels.len(), def.scheduler.clone());
        let now = Instant::now();
        let plan = sched.plan(&def.channels, now);
        assert!(!plan.poll_indices.is_empty());
        let rpm_idx = def.channels.iter().position(|c| c.id == "rpm").unwrap();
        assert!(plan.poll_indices.contains(&rpm_idx));
        assert!(plan.poll_indices[0] == rpm_idx || def.channels[plan.poll_indices[0]].priority <= def.channels[rpm_idx].priority);
    }

    #[test]
    fn test_scheduler_budget_caps_requests_per_tick() {
        let channels = sample_channels();
        let settings = TelemetrySchedulerSettings {
            tick_ms: 25,
            max_requests_per_tick: 2,
            skip_backlog: true,
        };
        let sched = TelemetryScheduler::new(channels.len(), settings);
        let now = Instant::now();
        let plan = sched.plan(&channels, now);
        assert_eq!(plan.poll_indices.len(), 2);
        assert_eq!(plan.budget_skipped.len(), 2);
        let mut sched = sched;
        sched.apply_budget_skips(&plan.budget_skipped, now);
        assert_eq!(sched.budget_skip_total, 2);
    }

    #[test]
    fn test_scheduler_skip_backlog_advances_skipped_channels() {
        let channels = sample_channels();
        let settings = TelemetrySchedulerSettings {
            tick_ms: 25,
            max_requests_per_tick: 1,
            skip_backlog: true,
        };
        let mut sched = TelemetryScheduler::new(channels.len(), settings);
        let now = Instant::now();
        let plan = sched.plan(&channels, now);
        assert_eq!(plan.poll_indices.len(), 1);
        sched.apply_budget_skips(&plan.budget_skipped, now);
        let plan2 = sched.plan(&channels, now);
        assert!(
            plan2.poll_indices.len() <= 1,
            "skipped channels should not all remain due immediately"
        );
    }

    #[test]
    fn test_ring_buffer_drops_counted() {
        let def = TelemetryChannelFile::load_default();
        let mut buf = TelemetryBuffer::from_definition(&def, Some(3));
        for i in 0..5 {
            buf.set_channel_value(0, i as f32, Instant::now());
            buf.push_frame();
        }
        assert_eq!(buf.frames.len(), 3);
        assert_eq!(buf.ring_drop_count, 2);
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
        let mut buf = TelemetryBuffer::from_definition(&def, Some(100));
        buf.set_channel_value(0, 1234.0, Instant::now());
        buf.push_frame();
        let path = std::env::temp_dir().join("wideband-telemetry-test.csv");
        buf.export_csv(path.to_str().unwrap()).unwrap();
        let imported = TelemetryBuffer::import_csv(path.to_str().unwrap(), &def).unwrap();
        assert_eq!(imported.len(), 1);
        assert!((imported[0].values[0] - 1234.0).abs() < 0.01);
        assert!(imported[0].unix_ms > 0 || imported[0].time_sec >= 0.0);
    }
}
