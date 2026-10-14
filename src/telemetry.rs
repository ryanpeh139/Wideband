use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::time::Instant;

#[derive(Clone, Copy, Debug, Default)]
pub struct TelemetrySnapshot {
    pub time_sec: f64,
    pub rpm: f32,
    pub map_kpa: f32,
    pub load_pct: f32,
    pub ect_c: f32,
    pub timing_deg: f32,
    pub iat_c: f32,
    pub tps_pct: f32,
    pub lambda: f32,
}

#[derive(Clone, Debug)]
pub struct TelemetryBuffer {
    pub snapshots: VecDeque<TelemetrySnapshot>,
    pub max_capacity: usize,
    pub is_logging: bool,
    pub start_time: Instant,
}

impl TelemetryBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            snapshots: VecDeque::with_capacity(capacity),
            max_capacity: capacity,
            is_logging: false,
            start_time: Instant::now(),
        }
    }

    pub fn push(&mut self, snap: TelemetrySnapshot) {
        if self.snapshots.len() >= self.max_capacity {
            self.snapshots.pop_front();
        }
        self.snapshots.push_back(snap);
    }

    pub fn latest(&self) -> TelemetrySnapshot {
        self.snapshots.back().copied().unwrap_or_default()
    }

    pub fn clear(&mut self) {
        self.snapshots.clear();
        self.start_time = Instant::now();
    }

    pub fn export_csv(&self, path: &str) -> Result<(), String> {
        let mut file = File::create(path).map_err(|e| format!("Failed to create CSV file: {}", e))?;
        writeln!(file, "Time_s,RPM,MAP_kPa,Load_pct,ECT_C,Timing_deg,IAT_C,TPS_pct,Lambda")
            .map_err(|e| format!("Failed to write header: {}", e))?;

        for s in &self.snapshots {
            writeln!(
                file,
                "{:.3},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1},{:.3}",
                s.time_sec, s.rpm, s.map_kpa, s.load_pct, s.ect_c, s.timing_deg, s.iat_c, s.tps_pct, s.lambda
            ).map_err(|e| format!("Failed to write row: {}", e))?;
        }
        Ok(())
    }

    pub fn import_csv(path: &str) -> Result<Vec<TelemetrySnapshot>, String> {
        let file = File::open(path).map_err(|e| format!("Failed to open CSV file: {}", e))?;
        let reader = BufReader::new(file);
        let mut result = Vec::new();

        for (idx, line) in reader.lines().enumerate() {
            let line = line.map_err(|e| format!("Error reading line: {}", e))?;
            if idx == 0 || line.trim().is_empty() {
                continue; // Skip header
            }

            let parts: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
            if parts.len() >= 9 {
                let snap = TelemetrySnapshot {
                    time_sec: parts[0].parse().unwrap_or(0.0),
                    rpm: parts[1].parse().unwrap_or(0.0),
                    map_kpa: parts[2].parse().unwrap_or(0.0),
                    load_pct: parts[3].parse().unwrap_or(0.0),
                    ect_c: parts[4].parse().unwrap_or(0.0),
                    timing_deg: parts[5].parse().unwrap_or(0.0),
                    iat_c: parts[6].parse().unwrap_or(0.0),
                    tps_pct: parts[7].parse().unwrap_or(0.0),
                    lambda: parts[8].parse().unwrap_or(1.0),
                };
                result.push(snap);
            }
        }
        Ok(result)
    }

    pub fn parse_mode01_response(snap: &mut TelemetrySnapshot, resp: &[u8]) {
        if resp.len() < 3 || resp[0] != 0x41 {
            return;
        }

        let pid = resp[1];
        match pid {
            0x0C => {
                // RPM: ((A * 256) + B) / 4
                if resp.len() >= 4 {
                    let raw = ((resp[2] as u16) << 8) | (resp[3] as u16);
                    snap.rpm = raw as f32 / 4.0;
                }
            }
            0x0B => {
                // MAP (kPa): A
                snap.map_kpa = resp[2] as f32;
            }
            0x04 => {
                // Calculated Engine Load %: A * 100 / 255
                snap.load_pct = (resp[2] as f32) * 100.0 / 255.0;
            }
            0x05 => {
                // ECT C: A - 40
                snap.ect_c = (resp[2] as f32) - 40.0;
            }
            0x0E => {
                // Timing Advance deg: (A / 2) - 64
                snap.timing_deg = ((resp[2] as f32) / 2.0) - 64.0;
            }
            0x0F => {
                // IAT C: A - 40
                snap.iat_c = (resp[2] as f32) - 40.0;
            }
            0x11 => {
                // Throttle Position %: A * 100 / 255
                snap.tps_pct = (resp[2] as f32) * 100.0 / 255.0;
            }
            0x24 | 0x34 => {
                // Wideband Lambda: ((A * 256) + B) / 32768
                if resp.len() >= 4 {
                    let raw = ((resp[2] as u16) << 8) | (resp[3] as u16);
                    snap.lambda = (raw as f32) / 32768.0;
                }
            }
            _ => {}
        }
    }
}
