use crate::ecu_bus;
use crate::j2534::PassThruDevice;
use crate::protocols::{DiagnosticProtocol, EcuProfile, VehicleBus};

/// Read-only emissions / OBD diagnostics. No clear-DTC, MIL defeat, or monitor disable paths.
pub struct DiagnosticsReader;

#[derive(Clone, Debug, Default)]
pub struct DiagnosticsSnapshot {
    pub mil_on: bool,
    pub stored_dtcs: Vec<DiagnosticTroubleCode>,
    pub pending_dtcs: Vec<DiagnosticTroubleCode>,
    pub readiness: ReadinessStatus,
    pub freeze_frames: Vec<FreezeFrame>,
    pub transport_note: String,
}

#[derive(Clone, Debug)]
pub struct DiagnosticTroubleCode {
    pub code: String,
    pub description: String,
    pub status: DtcStatus,
    pub raw: [u8; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DtcStatus {
    Confirmed,
    Pending,
    Permanent,
    Unknown,
}

#[derive(Clone, Debug)]
pub struct FreezeFrame {
    pub dtc_code: String,
    pub record_number: u8,
    pub pid_values: Vec<FreezePidValue>,
}

#[derive(Clone, Debug)]
pub struct FreezePidValue {
    pub pid: u8,
    pub label: String,
    pub value: String,
}

#[derive(Clone, Debug, Default)]
pub struct ReadinessStatus {
    pub monitors: Vec<ReadinessMonitor>,
}

#[derive(Clone, Debug)]
pub struct ReadinessMonitor {
    pub name: String,
    pub supported: bool,
    pub complete: bool,
}

impl DiagnosticsReader {
    pub fn read(
        device: &mut dyn PassThruDevice,
        channel: u32,
        profile: &EcuProfile,
    ) -> Result<DiagnosticsSnapshot, String> {
        let mut snap = DiagnosticsSnapshot::default();
        snap.transport_note = profile.name.clone();

        match profile.bus {
            VehicleBus::CanIsoTp => {
                if profile.diagnostic == DiagnosticProtocol::Uds {
                    Self::read_uds_obd_hybrid(device, channel, profile, &mut snap)?;
                } else {
                    Self::read_obd_modes(device, channel, profile, &mut snap)?;
                }
            }
            VehicleBus::KLine => {
                Self::read_obd_modes(device, channel, profile, &mut snap)?;
                if snap.stored_dtcs.is_empty() {
                    snap.transport_note =
                        format!("{} — K-Line: limited DTC support (OBD modes only)", profile.name);
                }
            }
            VehicleBus::Vpw => {
                Self::read_obd_modes(device, channel, profile, &mut snap)?;
                snap.transport_note =
                    format!("{} — J1850 VPW: OBD Mode $01/$03/$07 read path", profile.name);
            }
        }

        if snap.readiness.monitors.is_empty() {
            snap.readiness = default_readiness_unknown();
        }

        Ok(snap)
    }

    fn read_uds_obd_hybrid(
        device: &mut dyn PassThruDevice,
        channel: u32,
        profile: &EcuProfile,
        snap: &mut DiagnosticsSnapshot,
    ) -> Result<(), String> {
        Self::read_obd_modes(device, channel, profile, snap)?;

        let uds_dtcs = Self::uds_report_dtcs(device, channel, profile, 0xFF)?;
        for entry in uds_dtcs {
            if snap.stored_dtcs.iter().any(|d| d.code == entry.code) {
                continue;
            }
            snap.stored_dtcs.push(entry);
        }

        for dtc in &snap.stored_dtcs {
            if let Ok(ff) = Self::uds_freeze_frame(device, channel, profile, &dtc.raw, 0x01) {
                snap.freeze_frames.push(ff);
            }
        }
        Ok(())
    }

    fn read_obd_modes(
        device: &mut dyn PassThruDevice,
        channel: u32,
        profile: &EcuProfile,
        snap: &mut DiagnosticsSnapshot,
    ) -> Result<(), String> {
        if let Ok(resp) = Self::exchange(device, channel, profile, &[0x01, 0x01]) {
            if resp.len() >= 2 && resp[0] == 0x41 && resp[1] == 0x01 && resp.len() >= 6 {
                let a = resp[2];
                snap.mil_on = (a & 0x80) != 0;
                snap.readiness = parse_readiness_pid01(&resp[2..6]);
            }
        }

        if let Ok(resp) = Self::exchange(device, channel, profile, &[0x03]) {
            snap.stored_dtcs = parse_obd_dtc_list(&resp, 0x43, DtcStatus::Confirmed);
        }

        if let Ok(resp) = Self::exchange(device, channel, profile, &[0x07]) {
            snap.pending_dtcs = parse_obd_dtc_list(&resp, 0x47, DtcStatus::Pending);
        }

        if let Some(first) = snap.stored_dtcs.first() {
            if let Ok(resp) = Self::exchange(device, channel, profile, &[0x02, 0x02]) {
                if let Some(ff) = parse_obd_freeze_frame(&resp, &first.code) {
                    if !snap.freeze_frames.iter().any(|f| f.dtc_code == ff.dtc_code) {
                        snap.freeze_frames.push(ff);
                    }
                }
            }
        }
        Ok(())
    }

    fn uds_report_dtcs(
        device: &mut dyn PassThruDevice,
        channel: u32,
        profile: &EcuProfile,
        status_mask: u8,
    ) -> Result<Vec<DiagnosticTroubleCode>, String> {
        let req = vec![0x19, 0x02, status_mask];
        let resp = Self::exchange(device, channel, profile, &req)?;
        if resp.len() < 3 || resp[0] != 0x59 || resp[1] != 0x02 {
            return Err("UDS ReadDTCInformation (0x19 02) failed".to_string());
        }
        let mut out = Vec::new();
        let mut i = 4usize;
        while i + 3 < resp.len() {
            let raw = [resp[i], resp[i + 1]];
            let status_byte = resp[i + 3];
            let code = decode_dtc_bytes(raw[0], raw[1]);
            let status = uds_status_to_enum(status_byte);
            out.push(DiagnosticTroubleCode {
                code: code.clone(),
                description: dtc_description(&code),
                status,
                raw,
            });
            i += 4;
        }
        Ok(out)
    }

    fn uds_freeze_frame(
        device: &mut dyn PassThruDevice,
        channel: u32,
        profile: &EcuProfile,
        dtc_raw: &[u8; 2],
        record: u8,
    ) -> Result<FreezeFrame, String> {
        let req = vec![0x19, 0x04, dtc_raw[0], dtc_raw[1], 0xFF, record];
        let resp = Self::exchange(device, channel, profile, &req)?;
        if resp.len() < 6 || resp[0] != 0x59 || resp[1] != 0x04 {
            return Err("UDS snapshot record failed".to_string());
        }
        let code = decode_dtc_bytes(resp[2], resp[3]);
        let mut pid_values = Vec::new();
        let mut i = 5usize;
        while i + 1 < resp.len() {
            let pid = resp[i];
            let val_byte = resp[i + 1];
            pid_values.push(pid_to_display(pid, &[val_byte]));
            i += 2;
        }
        Ok(FreezeFrame {
            dtc_code: code,
            record_number: record,
            pid_values,
        })
    }

    fn exchange(
        device: &mut dyn PassThruDevice,
        channel: u32,
        profile: &EcuProfile,
        request: &[u8],
    ) -> Result<Vec<u8>, String> {
        ecu_bus::exchange(device, channel, profile, request)
    }
}

pub fn decode_dtc_bytes(high: u8, low: u8) -> String {
    let code = ((high as u16) << 8) | (low as u16);
    let prefix = match (code >> 14) & 0x03 {
        0 => 'P',
        1 => 'C',
        2 => 'B',
        _ => 'U',
    };
    format!("{}{:04X}", prefix, code & 0x3FFF)
}

fn uds_status_to_enum(status: u8) -> DtcStatus {
    if status & 0x08 != 0 {
        DtcStatus::Confirmed
    } else if status & 0x04 != 0 {
        DtcStatus::Pending
    } else if status & 0x10 != 0 {
        DtcStatus::Permanent
    } else {
        DtcStatus::Unknown
    }
}

fn parse_obd_dtc_list(resp: &[u8], positive: u8, default_status: DtcStatus) -> Vec<DiagnosticTroubleCode> {
    if resp.is_empty() || resp[0] != positive {
        return Vec::new();
    }
    let count = resp[1] as usize;
    let mut out = Vec::new();
    let mut i = 2usize;
    for _ in 0..count {
        if i + 1 >= resp.len() {
            break;
        }
        let raw = [resp[i], resp[i + 1]];
        let code = decode_dtc_bytes(raw[0], raw[1]);
        out.push(DiagnosticTroubleCode {
            code: code.clone(),
            description: dtc_description(&code),
            status: default_status,
            raw,
        });
        i += 2;
    }
    out
}

fn parse_obd_freeze_frame(resp: &[u8], dtc_code: &str) -> Option<FreezeFrame> {
    if resp.len() < 4 || resp[0] != 0x42 {
        return None;
    }
    let mut pid_values = Vec::new();
    let mut i = 2usize;
    while i < resp.len() {
        let pid = resp[i];
        if i + 1 >= resp.len() {
            break;
        }
        let nbytes = freeze_pid_len(pid);
        let end = (i + 1 + nbytes).min(resp.len());
        let slice = &resp[i + 1..end];
        pid_values.push(pid_to_display(pid, slice));
        i = end;
    }
    Some(FreezeFrame {
        dtc_code: dtc_code.to_string(),
        record_number: 0x02,
        pid_values,
    })
}

fn freeze_pid_len(pid: u8) -> usize {
    match pid {
        0x0C => 2,
        0x0E => 1,
        0x05 | 0x0B | 0x0F | 0x11 => 1,
        _ => 1,
    }
}

fn pid_to_display(pid: u8, data: &[u8]) -> FreezePidValue {
    let (label, value) = match pid {
        0x0C => {
            let raw = if data.len() >= 2 {
                u16::from_be_bytes([data[0], data[1]])
            } else {
                0
            };
            ("Engine RPM", format!("{:.0}", raw as f32 / 4.0))
        }
        0x05 => {
            let t = if data.is_empty() { 0 } else { data[0] };
            ("Coolant Temp", format!("{} °C", t as i16 - 40))
        }
        0x0B => {
            let kpa = if data.is_empty() { 0 } else { data[0] };
            ("MAP", format!("{} kPa", kpa))
        }
        0x0E => {
            let adv = if data.is_empty() { 0 } else { data[0] };
            ("Timing Advance", format!("{:.1}°", adv as f32 / 2.0 - 64.0))
        }
        0x11 => {
            let t = if data.is_empty() { 0 } else { data[0] };
            ("Throttle", format!("{:.1}%", t as f32 * 100.0 / 255.0))
        }
        _ => ("PID", format!("0x{:02X} = {}", pid, hex_bytes(data))),
    };
    FreezePidValue {
        pid,
        label: label.to_string(),
        value,
    }
}

fn hex_bytes(data: &[u8]) -> String {
    data.iter()
        .map(|b| format!("{:02X}", b))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn dtc_description(code: &str) -> String {
    match code {
        "P0171" => "System Too Lean (Bank 1)".to_string(),
        "P0300" => "Random/Multiple Cylinder Misfire Detected".to_string(),
        "P0420" => "Catalyst System Efficiency Below Threshold (Bank 1)".to_string(),
        "P0128" => "Coolant Thermostat (Coolant Temp Below Regulating Temperature)".to_string(),
        "C1201" => "Engine Control System Malfunction (Toyota network)".to_string(),
        _ => "Refer to OEM service information".to_string(),
    }
}

fn parse_readiness_pid01(bytes: &[u8]) -> ReadinessStatus {
    if bytes.len() < 4 {
        return default_readiness_unknown();
    }
    let b = bytes[1];
    let c = bytes[2];
    let d = bytes[3];

    let mut monitors = Vec::new();

    monitors.push(ReadinessMonitor {
        name: "Misfire".to_string(),
        supported: true,
        complete: (b & 0x10) != 0,
    });
    monitors.push(ReadinessMonitor {
        name: "Fuel System".to_string(),
        supported: true,
        complete: (b & 0x20) != 0,
    });
    monitors.push(ReadinessMonitor {
        name: "Components".to_string(),
        supported: true,
        complete: (b & 0x40) != 0,
    });

    let nct_supported = [
        ("Catalyst", 0x01u8, 0x01u8),
        ("Heated Catalyst", 0x02, 0x02),
        ("EVAP System", 0x04, 0x04),
        ("Secondary Air", 0x08, 0x08),
        ("A/C Refrigerant", 0x10, 0x10),
        ("Oxygen Sensor", 0x20, 0x20),
        ("Oxygen Sensor Heater", 0x40, 0x40),
        ("EGR / VVT System", 0x80, 0x80),
    ];

    for (name, sup_mask, comp_mask) in nct_supported {
        monitors.push(ReadinessMonitor {
            name: name.to_string(),
            supported: (c & sup_mask) != 0,
            complete: (d & comp_mask) != 0,
        });
    }

    ReadinessStatus { monitors }
}

fn default_readiness_unknown() -> ReadinessStatus {
    ReadinessStatus {
        monitors: vec![ReadinessMonitor {
            name: "Monitor data not available".to_string(),
            supported: false,
            complete: false,
        }],
    }
}

pub fn status_label(status: DtcStatus) -> &'static str {
    match status {
        DtcStatus::Confirmed => "Confirmed",
        DtcStatus::Pending => "Pending",
        DtcStatus::Permanent => "Permanent",
        DtcStatus::Unknown => "Unknown",
    }
}
