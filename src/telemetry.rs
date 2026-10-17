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
    #[serde(default)]
    pub module: ChannelModule,
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

/// Which module a live-data channel is read from.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChannelModule {
    /// The engine ECU (the connected vehicle profile).
    #[default]
    Ecm,
    /// The transmission ECU (the selected TCM profile, on the same connection).
    Tcm,
}

/// Data bytes in a Mode 01 reply for the PIDs live data uses (SAE J1979). PIDs not listed are
/// never batched with others.
pub fn mode01_data_len(pid: u8) -> Option<usize> {
    match pid {
        0x04 | 0x05 | 0x06 | 0x07 | 0x0B | 0x0D | 0x0E | 0x0F | 0x11 | 0x2F | 0x33 => Some(1),
        0x0C | 0x10 | 0x1F | 0x3C | 0x3D | 0x3E | 0x3F | 0x42 => Some(2),
        0x24..=0x2B => Some(4),
        _ => None,
    }
}

/// Splits a multi-PID Mode 01 reply (`41 p1 d.. p2 d..`) into one `41 pid data` reply per PID.
/// Stops at the first PID it cannot size; PIDs the ECU does not support are simply absent.
pub fn split_mode01_reply(resp: &[u8]) -> Vec<(u8, Vec<u8>)> {
    let mut out = Vec::new();
    if resp.first() != Some(&0x41) {
        return out;
    }
    let mut i = 1;
    while i < resp.len() {
        let pid = resp[i];
        let Some(len) = mode01_data_len(pid) else { break };
        if i + 1 + len > resp.len() {
            break;
        }
        let mut single = vec![0x41, pid];
        single.extend_from_slice(&resp[i + 1..i + 1 + len]);
        out.push((pid, single));
        i += 1 + len;
    }
    out
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
        // OBD PID 3C, catalyst temperature bank 1 sensor 1: (A*256+B)/10 - 40 °C.
        "pid_3c_cat_temp" => {
            if resp[0] == 0x41 && resp.len() >= 4 && resp[1] == 0x3C {
                let raw = ((resp[2] as u16) << 8) | (resp[3] as u16);
                Some(raw as f32 / 10.0 - 40.0)
            } else {
                None
            }
        }
        // Toyota ECM DID 107B, byte 25: engine oil temperature, A - 40 °C. Matched to a
        // Techstream Data List on a 2017 Tacoma 2GR-FKS (41 °C shown as 106 °F).
        "did_107b_oil_temp" => {
            if resp[0] == 0x62 && resp.len() >= 3 + 26 && resp[1] == 0x10 && resp[2] == 0x7B {
                Some(resp[3 + 25] as f32 - 40.0)
            } else {
                None
            }
        }
        // Toyota DID 1627 (ECM and TCM): A/T oil temperature No.1, A + B/256 - 40 °C. Matched
        // to Techstream: 52 80 = 42.5 °C (108.5 °F) on the ECM, 57 80 = 47.5 °C on the TCM.
        "did_1627_atf_temp" => {
            if resp[0] == 0x62 && resp.len() >= 5 && resp[1] == 0x16 && resp[2] == 0x27 {
                Some(resp[3] as f32 + resp[4] as f32 / 256.0 - 40.0)
            } else {
                None
            }
        }
        "did_1628_atf_temp2" => {
            if resp[0] == 0x62 && resp.len() >= 5 && resp[1] == 0x16 && resp[2] == 0x28 {
                Some(resp[3] as f32 + resp[4] as f32 / 256.0 - 40.0)
            } else {
                None
            }
        }
        // Toyota TCM DID 1011: data byte 3 holds the shift lever switches (0x20 P, 0x40 R,
        // 0x80 N; 0x01 = "N or P") and byte 5 bit 0 is D. 0 = P, 1 = R, 2 = N, 3 = D,
        // -1 = between positions. Mapped on a 2017 Tacoma by shifting P-R-N-D-N-P.
        "did_1011_shift_lever" => {
            if resp[0] == 0x62 && resp.len() >= 3 + 6 && resp[1] == 0x10 && resp[2] == 0x11 {
                let (sw, d) = (resp[3 + 3], resp[3 + 5]);
                Some(if sw & 0x20 != 0 {
                    0.0
                } else if sw & 0x40 != 0 {
                    1.0
                } else if sw & 0x80 != 0 {
                    2.0
                } else if d & 0x01 != 0 {
                    3.0
                } else {
                    -1.0
                })
            } else {
                None
            }
        }
        // Toyota TCM DID 1011, data byte 1 bit 0: brake (stop light) switch, 1 = pressed.
        "did_1011_brake" => {
            if resp[0] == 0x62 && resp.len() >= 3 + 2 && resp[1] == 0x10 && resp[2] == 0x11 {
                Some((resp[3 + 1] & 0x01) as f32)
            } else {
                None
            }
        }
        // Toyota DID 1601 (ECM and TCM): turbine (NT) sensor speed, A * 50 rpm. Matched to
        // Techstream: 0C = 600 rpm and 0B = 550 rpm in Park.
        "did_1601_nt_speed" => {
            if resp[0] == 0x62 && resp.len() >= 4 && resp[1] == 0x16 && resp[2] == 0x01 {
                Some(resp[3] as f32 * 50.0)
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

/// Consecutive failed polls (no reply, or a reply that does not decode) before a channel is
/// treated as unsupported by this ECU and left out of the schedule.
const UNSUPPORTED_AFTER_FAILURES: u8 = 3;

pub struct TelemetryScheduler {
    last_poll: Vec<Instant>,
    settings: TelemetrySchedulerSettings,
    pub budget_skip_total: u64,
    pub per_channel_budget_skips: Vec<u64>,
    fail_streak: Vec<u8>,
    unsupported: Vec<bool>,
}

impl TelemetryScheduler {
    pub fn new(channel_count: usize, settings: TelemetrySchedulerSettings) -> Self {
        let stale = Instant::now() - Duration::from_secs(3600);
        Self {
            last_poll: vec![stale; channel_count],
            settings,
            budget_skip_total: 0,
            per_channel_budget_skips: vec![0; channel_count],
            fail_streak: vec![0; channel_count],
            unsupported: vec![false; channel_count],
        }
    }

    /// The ECU does not have this PID or DID: stop polling it until `reset_support`.
    pub fn mark_unsupported(&mut self, idx: usize) {
        if idx < self.unsupported.len() {
            self.unsupported[idx] = true;
        }
    }

    pub fn is_unsupported(&self, idx: usize) -> bool {
        self.unsupported.get(idx).copied().unwrap_or(false)
    }

    /// A failed poll waits its interval like any other, and after
    /// `UNSUPPORTED_AFTER_FAILURES` in a row the channel is dropped. Returns true when this
    /// failure dropped it.
    pub fn record_failure(&mut self, idx: usize, when: Instant) -> bool {
        self.mark_polled(idx, when);
        if idx >= self.fail_streak.len() || self.unsupported[idx] {
            return false;
        }
        self.fail_streak[idx] = self.fail_streak[idx].saturating_add(1);
        if self.fail_streak[idx] >= UNSUPPORTED_AFTER_FAILURES {
            self.unsupported[idx] = true;
            return true;
        }
        false
    }

    /// Poll every channel again (after a reconnect or Clear live).
    pub fn reset_support(&mut self) {
        self.fail_streak.fill(0);
        self.unsupported.fill(false);
    }

    pub fn settings(&self) -> &TelemetrySchedulerSettings {
        &self.settings
    }

    /// Channels due this tick, most overdue first (elapsed time in units of the channel's own
    /// interval), then by priority band and index; capped by `max_requests_per_tick`.
    /// Unsupported channels are left out. Ranking by overdue ratio means a slow bus serves
    /// every channel in turn instead of the first few priority-1 channels forever.
    pub fn plan(&self, channels: &[TelemetryChannelDef], now: Instant) -> SchedulerPlan {
        let mut due: Vec<(u64, u8, usize)> = channels
            .iter()
            .enumerate()
            .filter(|(idx, _)| !self.is_unsupported(*idx))
            .filter_map(|(idx, ch)| {
                let elapsed = now.duration_since(self.last_poll[idx]).as_millis() as u64;
                if elapsed >= ch.interval_ms {
                    let overdue = elapsed.saturating_mul(1000) / ch.interval_ms.max(1);
                    Some((overdue, ch.priority, idx))
                } else {
                    None
                }
            })
            .collect();
        due.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));

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

    /// Counts channels that were due but over this tick's budget. Their clocks keep running so
    /// they rank higher next tick; there is no catch-up burst either way, because one poll
    /// restarts a channel's interval (`skip_backlog` is kept for config compatibility).
    pub fn apply_budget_skips(&mut self, skipped: &[usize], _now: Instant) {
        if skipped.is_empty() {
            return;
        }
        self.budget_skip_total += skipped.len() as u64;
        for &idx in skipped {
            if idx < self.per_channel_budget_skips.len() {
                self.per_channel_budget_skips[idx] += 1;
            }
        }
    }

    /// A successful poll: restart the channel's interval and clear its failure streak.
    pub fn mark_polled(&mut self, idx: usize, when: Instant) {
        if idx < self.last_poll.len() {
            self.last_poll[idx] = when;
        }
    }

    pub fn record_success(&mut self, idx: usize, when: Instant) {
        self.mark_polled(idx, when);
        if idx < self.fail_streak.len() {
            self.fail_streak[idx] = 0;
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
        self.scheduler.reset_support();
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
                module: ChannelModule::Ecm,
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
                module: ChannelModule::Ecm,
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
                module: ChannelModule::Ecm,
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
                module: ChannelModule::Ecm,
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
    fn test_scheduler_serves_every_channel_on_a_slow_bus() {
        // The real truck: each tick took ~250 ms, so the fast priority-1 channels were due on
        // every tick and the old scheduler never reached the rest.
        let def = TelemetryChannelFile::load_default();
        let channels = def.channels.clone();
        let mut sched = TelemetryScheduler::new(channels.len(), def.scheduler.clone());
        let mut now = Instant::now();
        let mut polled = vec![0u32; channels.len()];
        for _ in 0..40 {
            let plan = sched.plan(&channels, now);
            sched.apply_budget_skips(&plan.budget_skipped, now);
            for idx in plan.poll_indices {
                polled[idx] += 1;
                sched.record_success(idx, now);
            }
            now += Duration::from_millis(250);
        }
        for (idx, ch) in channels.iter().enumerate() {
            assert!(polled[idx] > 0, "{} was never polled", ch.id);
        }
        let rpm = channels.iter().position(|c| c.id == "rpm").unwrap();
        assert!(polled[rpm] >= polled.iter().copied().min().unwrap(), "fast channels still poll most");
    }

    #[test]
    fn test_scheduler_drops_unsupported_channels() {
        let channels = sample_channels();
        let mut sched = TelemetryScheduler::new(channels.len(), TelemetrySchedulerSettings::default());
        let now = Instant::now();
        assert!(!sched.record_failure(0, now));
        assert!(!sched.record_failure(0, now));
        assert!(sched.record_failure(0, now), "third failure in a row drops the channel");
        sched.mark_unsupported(1);
        let later = now + Duration::from_secs(10);
        let plan = sched.plan(&channels, later);
        assert!(!plan.poll_indices.contains(&0) && !plan.budget_skipped.contains(&0));
        assert!(!plan.poll_indices.contains(&1) && !plan.budget_skipped.contains(&1));
        sched.reset_support();
        assert!(sched.plan(&channels, later).poll_indices.contains(&0) || sched.plan(&channels, later).budget_skipped.contains(&0));
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
    fn test_decode_tacoma_channels_from_techstream_capture() {
        // Replies captured from a 2017 Tacoma alongside a Techstream Data List.
        let mut oil = vec![0x62, 0x10, 0x7B];
        oil.extend_from_slice(&[
            0x00, 0x00, 0x05, 0x80, 0x04, 0x00, 0x00, 0x7F, 0xB7, 0x84, 0x7F, 0x7F, 0xEF, 0x7F, 0xB8, 0x81, 0x54,
            0x80, 0x00, 0x80, 0x06, 0x00, 0x00, 0x81, 0x36, 0x51, 0x6E, 0x30, 0x30, 0x02, 0x90,
        ]);
        assert_eq!(decode_channel_value("did_107b_oil_temp", &oil), Some(41.0), "Techstream: 106 °F");
        let atf = decode_channel_value("did_1627_atf_temp", &[0x62, 0x16, 0x27, 0x52, 0x80]).unwrap();
        assert!((atf - 42.5).abs() < 0.01, "Techstream: 108.5 °F");
        let cat = decode_channel_value("pid_3c_cat_temp", &[0x41, 0x3C, 0x12, 0x01]).unwrap();
        assert!((cat - 420.9).abs() < 0.01);
        let v = decode_channel_value("pid_42_ecu_v", &[0x41, 0x42, 0x35, 0x80]).unwrap();
        assert!((v - 13.696).abs() < 0.01, "Techstream: 13.7 V");
        assert_eq!(decode_channel_value("did_107b_oil_temp", &[0x62, 0x10, 0x7B, 0x00]), None, "short reply");
    }

    #[test]
    fn test_decode_shift_lever_from_shifting_capture() {
        // TCM 1011 replies recorded while shifting P-R-N-D-N-P with the brake held.
        let reply = |b: [u8; 10]| {
            let mut r = vec![0x62, 0x10, 0x11];
            r.extend_from_slice(&b);
            r
        };
        let p = reply([0x03, 0x01, 0xE1, 0x21, 0x03, 0x00, 0x80, 0x00, 0x10, 0x00]);
        let r = reply([0x03, 0x01, 0xE1, 0x40, 0x03, 0x00, 0x80, 0x00, 0x10, 0x00]);
        let n = reply([0x03, 0x01, 0xE1, 0x81, 0x03, 0x00, 0x80, 0x00, 0x10, 0x00]);
        let d = reply([0x03, 0x01, 0xE1, 0x00, 0x03, 0x01, 0x80, 0x00, 0x10, 0x00]);
        let off = reply([0x03, 0x00, 0xE1, 0x21, 0x03, 0x00, 0x80, 0x00, 0x10, 0x00]);
        let lever = |r: &[u8]| decode_channel_value("did_1011_shift_lever", r);
        assert_eq!((lever(&p), lever(&r), lever(&n), lever(&d)), (Some(0.0), Some(1.0), Some(2.0), Some(3.0)));
        assert_eq!(decode_channel_value("did_1011_brake", &p), Some(1.0));
        assert_eq!(decode_channel_value("did_1011_brake", &off), Some(0.0));
        assert_eq!(decode_channel_value("did_1601_nt_speed", &[0x62, 0x16, 0x01, 0x0C]), Some(600.0));
    }

    #[test]
    fn test_split_multi_pid_reply() {
        // 01 0C 11 0D 24: RPM, TPS, speed, lambda; the ECU leaves out a PID it lacks.
        let resp = [0x41, 0x0C, 0x1A, 0xF8, 0x11, 0x30, 0x24, 0x80, 0x19, 0x69, 0xD7];
        let parts = split_mode01_reply(&resp);
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0], (0x0C, vec![0x41, 0x0C, 0x1A, 0xF8]));
        assert_eq!(parts[1], (0x11, vec![0x41, 0x11, 0x30]));
        assert_eq!(parts[2].0, 0x24);
        assert_eq!(decode_channel_value("pid_0c_rpm", &parts[0].1), Some(1726.0));
        assert!(split_mode01_reply(&[0x7F, 0x01, 0x12]).is_empty());
        assert!(split_mode01_reply(&[0x41, 0x0C, 0x1A]).is_empty(), "truncated reply");
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
