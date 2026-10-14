use std::time::Instant;

#[cfg(windows)]
use std::ffi::c_void;

pub const STATUS_NOERROR: i32 = 0;
pub const J1850VPW: u32 = 1;
pub const J1850PWM: u32 = 2;
pub const ISO9141: u32 = 3;
pub const ISO14230: u32 = 4;
pub const CAN: u32 = 6;
pub const ISO15765: u32 = 5;

pub const CAN_29BIT_ID: u32 = 0x00000100;
pub const ISO15765_FRAME_PAD: u32 = 0x00000040;

pub const PASS_FILTER: u32 = 1;
pub const BLOCK_FILTER: u32 = 2;
pub const FLOW_CONTROL_FILTER: u32 = 3;

pub const GET_CONFIG: u32 = 1;
pub const SET_CONFIG: u32 = 2;
pub const READ_VBATT: u32 = 3;
pub const FIVE_BAUD_INIT: u32 = 4;
pub const FAST_INIT: u32 = 5; // ISO 14230 fast init ioctl (SAE J2534-1; not READ_VBATT)
pub const CLEAR_TX_BUFFER: u32 = 7;
pub const CLEAR_RX_BUFFER: u32 = 8;

pub const ISO15765_BS: u32 = 0x10;
pub const ISO15765_STMIN: u32 = 0x11;
pub const DATA_RATE: u32 = 0x01;
pub const LOOPBACK: u32 = 0x03;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct PassThruMsg {
    pub protocol_id: u32,
    pub rx_status: u32,
    pub tx_flags: u32,
    pub timestamp: u32,
    pub data_size: u32,
    pub extra_data_index: u32,
    pub data: [u8; 4128],
}

impl Default for PassThruMsg {
    fn default() -> Self {
        Self {
            protocol_id: 0,
            rx_status: 0,
            tx_flags: 0,
            timestamp: 0,
            data_size: 0,
            extra_data_index: 0,
            data: [0u8; 4128],
        }
    }
}

impl PassThruMsg {
    pub fn new(protocol: u32, payload: &[u8]) -> Self {
        let mut msg = Self::default();
        msg.protocol_id = protocol;
        let len = payload.len().min(4128);
        msg.data_size = len as u32;
        msg.data[..len].copy_from_slice(&payload[..len]);
        msg
    }

    pub fn new_can(can_id: u32, payload: &[u8]) -> Self {
        let mut msg = Self::default();
        msg.protocol_id = ISO15765;
        msg.write_id(can_id);
        let len = payload.len().min(4124);
        msg.data_size = (4 + len) as u32;
        msg.data[4..4 + len].copy_from_slice(&payload[..len]);
        msg
    }

    /// Big-Endian CAN ID in data[0..4] per SAE J2534-1
    pub fn write_id(&mut self, id: u32) {
        self.data[0] = ((id >> 24) & 0xFF) as u8;
        self.data[1] = ((id >> 16) & 0xFF) as u8;
        self.data[2] = ((id >> 8) & 0xFF) as u8;
        self.data[3] = (id & 0xFF) as u8;
    }

    /// Read Big-Endian CAN ID from data[0..4]
    pub fn read_id(&self) -> u32 {
        ((self.data[0] as u32) << 24)
            | ((self.data[1] as u32) << 16)
            | ((self.data[2] as u32) << 8)
            | (self.data[3] as u32)
    }

    pub fn payload(&self) -> &[u8] {
        if self.protocol_id == ISO15765 || self.protocol_id == CAN {
            if self.data_size >= 4 {
                &self.data[4..self.data_size as usize]
            } else {
                &[]
            }
        } else {
            &self.data[..self.data_size as usize]
        }
    }
}

/// Bench Tacoma identifiers — obviously synthetic (VIN contains `VIRTUAL`).
pub const VIRTUAL_TACOMA_VIN: &str = "5TFRX54N0VIRT0001";
pub const VIRTUAL_TACOMA_CAL_ID: &str = "89663-VIRT01";
pub const VIRTUAL_TACOMA_TCM_CAL_ID: &str = "89662-VIRT01";
pub const VIRTUAL_SILVERADO_TCM_CAL_ID: &str = "24268164-VIRT";
pub const VIRTUAL_TACOMA_CVN_HEX: &str = "DEADBEEF";

pub fn virtual_tcm_cal_id_for_profile(profile_id: &str) -> String {
    if profile_id.contains("silverado") {
        VIRTUAL_SILVERADO_TCM_CAL_ID.to_string()
    } else {
        VIRTUAL_TACOMA_TCM_CAL_ID.to_string()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BenchEcuRole {
    Ecm,
    Tcm,
}

fn bench_rx_id_for_tx(tx_id: u32) -> (u32, BenchEcuRole) {
    match tx_id {
        0x7E1 => (0x7E9, BenchEcuRole::Tcm),
        0x7E0 => (0x7E8, BenchEcuRole::Ecm),
        0x18DA28F1 => (0x18DAF128, BenchEcuRole::Tcm),
        0x18DA10F1 => (0x18DAF110, BenchEcuRole::Ecm),
        _ => (0x7E8, BenchEcuRole::Ecm),
    }
}

#[derive(Clone, Debug)]
pub struct J2534DeviceInfo {
    pub name: String,
    pub vendor: String,
    pub library_path: String,
    pub is_virtual: bool,
    pub exists: bool,
}

impl J2534DeviceInfo {
    /// Dropdown label: kind prefix + device name (+ missing DLL hint).
    pub fn list_label(&self) -> String {
        if self.is_virtual {
            return "[Virtual] Offline loopback ECU (practice ladder)".to_string();
        }
        let kind = classify_pass_thru_kind(&self.name, &self.vendor, &self.library_path);
        let missing = if !self.exists { " — DLL not found" } else { "" };
        format!("[{}] {}{}", kind, self.name, missing)
    }
}

fn classify_pass_thru_kind(name: &str, vendor: &str, library_path: &str) -> &'static str {
    let hay = format!("{} {} {}", name, vendor, library_path).to_lowercase();
    if hay.contains("mini") && (hay.contains("vci") || hay.contains("vxdiag") || hay.contains("xhorse")) {
        "Mini-VCI J2534"
    } else if hay.contains("vxdiag") || hay.contains("vcx") || hay.contains("nano") {
        "VXDIAG / VCX Nano J2534"
    } else if hay.contains("tactrix") || hay.contains("openport") {
        "Tactrix J2534"
    } else if hay.contains("scanmatik") || hay.contains("mongoose") {
        "Scanmatik / Mongoose J2534"
    } else if hay.contains("esp32") || hay.contains("vf tuner") {
        "VF Tuner J2534"
    } else {
        "J2534 Pass-Thru"
    }
}

pub struct J2534RegistryScanner;

impl J2534RegistryScanner {
    pub fn scan() -> Vec<J2534DeviceInfo> {
        let mut devices = Vec::new();

        // Always include our Virtual ECU Simulator loopback device
        devices.push(J2534DeviceInfo {
            name: "Wideband Virtual ECU Simulator (Loopback)".to_string(),
            vendor: "Wideband Bench Suite".to_string(),
            library_path: "virtual".to_string(),
            is_virtual: true,
            exists: true,
        });

        #[cfg(target_os = "windows")]
        {
            use winreg::enums::*;
            use winreg::RegKey;

            let roots = [
                r"SOFTWARE\WOW6432Node\PassThruSupport.04.04",
                r"SOFTWARE\PassThruSupport.04.04",
            ];

            let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
            for root_path in &roots {
                if let Ok(root) = hklm.open_subkey_with_flags(root_path, KEY_READ) {
                    for key_name in root.enum_keys().filter_map(|r| r.ok()) {
                        if let Ok(dev_key) = root.open_subkey_with_flags(&key_name, KEY_READ) {
                            let name: String = dev_key.get_value("Name").unwrap_or_else(|_| key_name.clone());
                            let vendor: String = dev_key.get_value("Vendor").unwrap_or_else(|_| "Unknown Vendor".to_string());
                            let lib_path: String = dev_key.get_value("FunctionLibrary").unwrap_or_default();
                            if !lib_path.is_empty() {
                                let exists = std::path::Path::new(&lib_path).exists();
                                if !devices.iter().any(|d| d.library_path.eq_ignore_ascii_case(&lib_path)) {
                                    devices.push(J2534DeviceInfo {
                                        name,
                                        vendor,
                                        library_path: lib_path,
                                        is_virtual: false,
                                        exists,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }

        let fallbacks = known_passthru_dll_paths();
        for (name, vendor, path) in &fallbacks {
            push_dll_device(&mut devices, name, vendor, path);
        }

        #[cfg(target_os = "windows")]
        scan_install_folder_dlls(&mut devices, r"C:\Program Files (x86)\VXDIAG", "VXDIAG");
        #[cfg(target_os = "windows")]
        scan_install_folder_dlls(&mut devices, r"C:\Program Files (x86)\MVCI", "Mini-VCI / MVCI");
        #[cfg(target_os = "windows")]
        scan_install_folder_dlls(&mut devices, r"C:\Program Files (x86)\Mini VCI", "Mini-VCI");
        #[cfg(target_os = "windows")]
        scan_install_folder_dlls(&mut devices, r"C:\Program Files (x86)\XHorse", "XHorse MVCI");

        devices
    }
}

fn push_dll_device(devices: &mut Vec<J2534DeviceInfo>, name: &str, vendor: &str, path: &str) {
    if std::path::Path::new(path).exists()
        && !devices.iter().any(|d| d.library_path.eq_ignore_ascii_case(path))
    {
        devices.push(J2534DeviceInfo {
            name: name.to_string(),
            vendor: vendor.to_string(),
            library_path: path.to_string(),
            is_virtual: false,
            exists: true,
        });
    }
}

/// Well-known PassThru DLL paths when registry keys are missing or mis-pointed.
pub(crate) fn known_passthru_dll_paths() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        (
            "Tactrix OpenPort 2.0",
            "Tactrix Inc.",
            r"C:\Windows\System32\op20pt32.dll",
        ),
        (
            "Tactrix OpenPort 2.0 (SysWOW64)",
            "Tactrix Inc.",
            r"C:\Windows\SysWOW64\op20pt32.dll",
        ),
        (
            "VF Tuner ESP32 Pass-Thru",
            "VF Tuner",
            r"C:\Program Files (x86)\VF Tuner\esp32.dll",
        ),
        (
            "Scanmatik SM2 Pass-Thru",
            "Scanmatik",
            r"C:\Program Files (x86)\Scanmatik\sm2pt32.dll",
        ),
        (
            "Mini-VCI J2534 (MVCI PassThruPro)",
            "MVCI / XHorse",
            r"C:\Program Files (x86)\MVCI\J2534\PassThruPro.dll",
        ),
        (
            "Mini-VCI J2534 (alternate)",
            "MVCI",
            r"C:\Program Files (x86)\MVCI\PassThruPro.dll",
        ),
        (
            "VXDIAG VCX Nano Toyota J2534",
            "VXDIAG",
            r"C:\Program Files (x86)\VXDIAG\VcxNv_Toyota\VCIToyota32.dll",
        ),
        (
            "VXDIAG VCX Toyota PassThru (Pt32)",
            "VXDIAG",
            r"C:\Program Files (x86)\VXDIAG\VcxNv_Toyota\Pt32Toyota.dll",
        ),
        (
            "VXDIAG VCX Nano (generic)",
            "VXDIAG",
            r"C:\Program Files (x86)\VXDIAG\VCXNANO\VCXNANO32.dll",
        ),
        (
            "VXDIAG VCX Manager Toyota J2534",
            "VXDIAG",
            r"C:\Program Files\VXDIAG\VcxNv_Toyota_J2534\Pt32Toyota.dll",
        ),
    ]
}

#[cfg(target_os = "windows")]
fn scan_install_folder_dlls(devices: &mut Vec<J2534DeviceInfo>, root: &str, vendor_label: &str) {
    let root_path = std::path::Path::new(root);
    if root_path.is_dir() {
        scan_install_folder_dlls_inner(devices, root_path, vendor_label, 0);
    }
}

#[cfg(target_os = "windows")]
fn scan_install_folder_dlls_inner(
    devices: &mut Vec<J2534DeviceInfo>,
    dir: &std::path::Path,
    vendor_label: &str,
    depth: usize,
) {
    if depth > 4 {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_install_folder_dlls_inner(devices, &path, vendor_label, depth + 1);
            continue;
        }
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !file_name.ends_with(".dll") {
            continue;
        }
        let looks_j2534 = file_name.contains("passthru")
            || file_name.contains("j2534")
            || file_name.contains("pt32")
            || file_name.contains("vci")
            || file_name.contains("vcx");
        if !looks_j2534 {
            continue;
        }
        let path_str = path.to_string_lossy().to_string();
        if devices.iter().any(|d| d.library_path.eq_ignore_ascii_case(&path_str)) {
            continue;
        }
        let display = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("J2534 Pass-Thru");
        devices.push(J2534DeviceInfo {
            name: format!("{} ({})", vendor_label, display),
            vendor: vendor_label.to_string(),
            library_path: path_str,
            is_virtual: false,
            exists: true,
        });
    }
}

#[cfg(not(target_os = "windows"))]
fn scan_install_folder_dlls(_devices: &mut Vec<J2534DeviceInfo>, _root: &str, _vendor_label: &str) {}

pub trait PassThruDevice: Send + Sync {
    fn open(&mut self) -> Result<(), String>;
    fn close(&mut self) -> Result<(), String>;
    fn connect(&mut self, protocol: u32, flags: u32, baud: u32) -> Result<u32, String>;
    fn disconnect(&mut self, channel: u32) -> Result<(), String>;
    fn read_msgs(&mut self, channel: u32, timeout_ms: u32) -> Result<Vec<PassThruMsg>, String>;
    fn write_msgs(&mut self, channel: u32, msgs: &[PassThruMsg], timeout_ms: u32) -> Result<u32, String>;
    fn start_filter(&mut self, channel: u32, filter_type: u32, mask: &PassThruMsg, pattern: &PassThruMsg, flow_control: Option<&PassThruMsg>) -> Result<u32, String>;
    fn ioctl(&mut self, channel: u32, ioctl_id: u32, input: &[u8]) -> Result<Vec<u8>, String>;
    fn is_connected(&self) -> bool;
    fn firmware_version(&self) -> String;
    fn channel_id(&self) -> u32 {
        1
    }
    fn device_label(&self) -> String {
        "unknown".to_string()
    }
    /// Host bridge uses FILTER during CONNECT; native uses this before filters.
    fn set_connect_hints(&mut self, _extended: bool, _filter_id: u32, _filter_mask: u32) {}

    /// ISO 14230 fast init after connect (K-Line). Default uses `FAST_INIT` ioctl.
    fn kline_fast_init(&mut self, channel: u32, frame: &[u8]) -> Result<Vec<u8>, String> {
        self.ioctl(channel, FAST_INIT, frame)
    }

    /// Battery voltage from PassThru READ_VBATT ioctl (mV). None if unavailable.
    fn read_vbatt_volts(&mut self) -> Option<f32> {
        let ch = self.channel_id();
        match self.ioctl(ch, READ_VBATT, &[]) {
            Ok(out) if out.len() >= 4 => {
                let mv = u32::from_le_bytes([out[0], out[1], out[2], out[3]]);
                if mv >= 9_000 && mv <= 20_000 {
                    Some(mv as f32 / 1000.0)
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

/// In-memory loopback ECU simulator for bench testing without vehicle hardware
pub struct VirtualEcuSimulator {
    connected: bool,
    channel_id: u32,
    active_protocol: u32,
    rx_queue: Vec<PassThruMsg>,
    start_time: Instant,
    unlocked_level1: bool,
    pending_security_seed: [u8; 4],
    download_active: bool,
    transfer_offset: u32,
    received_bytes: Vec<u8>,
    rx_assembly: Vec<u8>,
    rx_expected_len: usize,
}

impl VirtualEcuSimulator {
    pub fn new() -> Self {
        Self {
            connected: false,
            channel_id: 1,
            active_protocol: ISO15765,
            rx_queue: Vec::new(),
            start_time: Instant::now(),
            unlocked_level1: false,
            pending_security_seed: [0x48, 0x54, 0x69, 0x76],
            download_active: false,
            transfer_offset: 0,
            received_bytes: Vec::new(),
            rx_assembly: Vec::new(),
            rx_expected_len: 0,
        }
    }

    fn generate_telemetry_pid(&self, pid: u8) -> Vec<u8> {
        let elapsed = self.start_time.elapsed().as_secs_f32();
        // Dynamic oscillations simulating a running engine
        match pid {
            0x0C => {
                // RPM: 800 + sine wave up to 4500 RPM
                let rpm = 800.0 + (elapsed * 0.8).sin().abs() * 3700.0;
                let raw = (rpm * 4.0) as u16;
                vec![0x41, 0x0C, (raw >> 8) as u8, (raw & 0xFF) as u8]
            }
            0x0B => {
                // MAP (kPa): 35 kPa idle, up to 160 kPa under simulated load
                let map = 35.0 + (elapsed * 0.5).sin().abs() * 120.0;
                vec![0x41, 0x0B, map as u8]
            }
            0x04 => {
                // Calculated Engine Load %: 18% to 85%
                let load = 18.0 + (elapsed * 0.6).sin().abs() * 65.0;
                let raw = (load * 255.0 / 100.0) as u8;
                vec![0x41, 0x04, raw]
            }
            0x05 => {
                // Coolant Temp C: 88 to 92 C (stored as C + 40)
                let ect = 89;
                vec![0x41, 0x05, (ect + 40) as u8]
            }
            0x0E => {
                // Timing advance: 15 to 32 degrees (formula: (raw / 2) - 64)
                let timing = 18.0 + (elapsed * 0.4).cos().abs() * 14.0;
                let raw = ((timing + 64.0) * 2.0) as u8;
                vec![0x41, 0x0E, raw]
            }
            0x0F => {
                // IAT C: 28 C (formula: raw - 40)
                let iat = 28;
                vec![0x41, 0x0F, (iat + 40) as u8]
            }
            0x11 => {
                // Throttle %: 12% to 75%
                let tps = 12.0 + (elapsed * 0.7).sin().abs() * 60.0;
                let raw = (tps * 255.0 / 100.0) as u8;
                vec![0x41, 0x11, raw]
            }
            0x24 => {
                // Wideband Lambda: 0.84 to 1.02
                let lambda = 0.88 + (elapsed * 0.3).sin().abs() * 0.14;
                let raw = (lambda * 32768.0) as u16;
                vec![0x41, 0x24, (raw >> 8) as u8, (raw & 0xFF) as u8, 0x80, 0x00]
            }
            _ => vec![0x7F, 0x01, 0x12], // Sub-function not supported
        }
    }

    fn enqueue_iso_tp_response(&mut self, rx_id: u32, resp_payload: &[u8]) {
        let len = resp_payload.len();
        if len <= 7 {
            let mut b = vec![len as u8];
            b.extend_from_slice(resp_payload);
            while b.len() < 8 {
                b.push(0xAA);
            }
            let mut resp_msg = PassThruMsg::new_can(rx_id, &b);
            resp_msg.protocol_id = self.active_protocol;
            self.rx_queue.push(resp_msg);
            return;
        }

        let mut ff = vec![0x10 | ((len >> 8) as u8 & 0x0F), (len & 0xFF) as u8];
        ff.extend_from_slice(&resp_payload[..6.min(len)]);
        while ff.len() < 8 {
            ff.push(0xAA);
        }
        let mut ff_msg = PassThruMsg::new_can(rx_id, &ff);
        ff_msg.protocol_id = self.active_protocol;
        self.rx_queue.push(ff_msg);

        let mut offset = 6;
        let mut seq = 1u8;
        while offset < len {
            let chunk = (len - offset).min(7);
            let mut cf = vec![0x20 | (seq & 0x0F)];
            cf.extend_from_slice(&resp_payload[offset..offset + chunk]);
            while cf.len() < 8 {
                cf.push(0xAA);
            }
            let mut cf_msg = PassThruMsg::new_can(rx_id, &cf);
            cf_msg.protocol_id = self.active_protocol;
            self.rx_queue.push(cf_msg);
            offset += chunk;
            seq = (seq + 1) & 0x0F;
        }
    }

    fn handle_security_access(&mut self, req: &[u8]) -> Vec<u8> {
        if req.len() < 2 {
            return vec![0x7F, 0x27, 0x13];
        }
        let sub = req[1];
        if sub == 0x01 {
            if self.unlocked_level1 {
                return vec![0x67, 0x01, 0x00, 0x00, 0x00, 0x00];
            }
            let seed = self.pending_security_seed;
            vec![0x67, 0x01, seed[0], seed[1], seed[2], seed[3]]
        } else if sub == 0x02 {
            if req.len() < 6 {
                return vec![0x7F, 0x27, 0x13];
            }
            let key = &req[2..6];
            match crate::protocols::SecurityKeys::compute_denso32(
                &self.pending_security_seed,
                0,
                true,
            ) {
                Ok(expected) if key == expected.as_slice() => {
                    self.unlocked_level1 = true;
                    vec![0x67, 0x02]
                }
                _ => vec![0x7F, 0x27, 0x35],
            }
        } else {
            vec![0x7F, 0x27, 0x12]
        }
    }

    fn handle_uds_request(&mut self, req: &[u8], role: BenchEcuRole) -> Vec<u8> {
        if req.is_empty() {
            return vec![0x7F, 0x00, 0x13];
        }

        match req[0] {
            // Mode 01: Current Powertrain Diagnostic Data
            0x01 => {
                if req.len() >= 2 {
                    if req[1] == 0x01 {
                        // MIL on, 1 stored DTC; catalyst incomplete, others complete (bench)
                        vec![0x41, 0x01, 0x81, 0x07, 0xE5, 0x9D]
                    } else {
                        self.generate_telemetry_pid(req[1])
                    }
                } else {
                    vec![0x7F, 0x01, 0x13]
                }
            }
            // Mode 02: Freeze frame
            0x02 => {
                if req.len() >= 2 && req[1] == 0x02 {
                    vec![
                        0x42, 0x02, 0x01, 0x71, 0x0C, 0x1A, 0x20, 0x05, 0x58, 0x0E, 0x8A, 0x11, 0x2A,
                    ]
                } else {
                    vec![0x7F, 0x02, 0x12]
                }
            }
            // Mode 03: Stored DTCs
            0x03 => vec![0x43, 0x01, 0x01, 0x71],
            // Mode 07: Pending DTCs
            0x07 => vec![0x47, 0x01, 0x03, 0x00],
            // Mode 09: Vehicle Info
            0x09 => {
                if req.len() >= 2 {
                    match req[1] {
                        0x02 => {
                            let mut resp = vec![0x49, 0x02, 0x01];
                            resp.extend_from_slice(VIRTUAL_TACOMA_VIN.as_bytes());
                            resp
                        }
                        0x04 => {
                            let mut resp = vec![0x49, 0x04, 0x01];
                            resp.extend_from_slice(VIRTUAL_TACOMA_CAL_ID.as_bytes());
                            resp
                        }
                        0x06 => {
                            vec![0x49, 0x06, 0x01, 0xDE, 0xAD, 0xBE, 0xEF]
                        }
                        _ => vec![0x7F, 0x09, 0x12],
                    }
                } else {
                    vec![0x7F, 0x09, 0x13]
                }
            }
            // 0x10: DiagnosticSessionControl
            0x10 => {
                let session = if req.len() > 1 { req[1] } else { 0x01 };
                vec![0x50, session, 0x00, 0x32, 0x01, 0xF4] // P2 = 50ms, P2* = 5000ms
            }
            // 0x3E: TesterPresent
            0x3E => {
                vec![0x7E, if req.len() > 1 { req[1] } else { 0x00 }]
            }
            // 0x22: ReadDataByIdentifier
            0x22 => {
                if req.len() >= 3 {
                    let did = ((req[1] as u16) << 8) | (req[2] as u16);
                    match did {
                        0xF190 => {
                            let mut resp = vec![0x62, 0xF1, 0x90];
                            resp.extend_from_slice(VIRTUAL_TACOMA_VIN.as_bytes());
                            resp
                        }
                        0xF181 => {
                            let mut resp = vec![0x62, 0xF1, 0x81];
                            resp.extend_from_slice(VIRTUAL_TACOMA_CAL_ID.as_bytes());
                            resp
                        }
                        0xF182 => {
                            let mut resp = vec![0x62, 0xF1, 0x82];
                            resp.extend_from_slice(VIRTUAL_TACOMA_TCM_CAL_ID.as_bytes());
                            resp
                        }
                        0x00C1 => {
                            // GM ECM Cal ID: 12619623
                            let mut resp = vec![0x62, 0x00, 0xC1];
                            resp.extend_from_slice(b"12619623");
                            resp
                        }
                        0x00C2 => {
                            let mut resp = vec![0x62, 0x00, 0xC2];
                            let cal = if role == BenchEcuRole::Tcm {
                                VIRTUAL_SILVERADO_TCM_CAL_ID
                            } else {
                                "24268163"
                            };
                            resp.extend_from_slice(cal.as_bytes());
                            resp
                        }
                        0xF010 => {
                            // Bench battery voltage (mV) — ~13.8 V with mild ripple
                            let elapsed = self.start_time.elapsed().as_secs_f32();
                            let vbatt = 13.2 + (elapsed * 0.2).sin().abs() * 0.8;
                            let mv = (vbatt * 1000.0) as u16;
                            vec![0x62, 0xF0, 0x10, (mv >> 8) as u8, (mv & 0xFF) as u8]
                        }
                        0xF011 => {
                            // Bench oil temp °C (A - 40)
                            let elapsed = self.start_time.elapsed().as_secs_f32();
                            let oil = 85.0 + (elapsed * 0.15).sin() * 8.0;
                            vec![0x62, 0xF0, 0x11, (oil + 40.0) as u8]
                        }
                        _ => vec![0x7F, 0x22, 0x31], // Request out of range
                    }
                } else {
                    vec![0x7F, 0x22, 0x13]
                }
            }
            // 0x27: SecurityAccess
            0x27 => self.handle_security_access(req),
            // 0x34: RequestDownload
            0x34 => {
                if !self.unlocked_level1 {
                    return vec![0x7F, 0x34, 0x33];
                }
                self.download_active = true;
                self.transfer_offset = 0;
                self.received_bytes.clear();
                // 0x74 + lengthFormat (0x20) + maxNumberOfBlockLength (0x0802 = 2050 bytes)
                vec![0x74, 0x20, 0x08, 0x02]
            }
            // 0x36: TransferData
            0x36 => {
                if !self.unlocked_level1 {
                    return vec![0x7F, 0x36, 0x33];
                }
                if !self.download_active {
                    return vec![0x7F, 0x36, 0x24]; // Request Sequence Error
                }
                let seq = if req.len() > 1 { req[1] } else { 0 };
                if req.len() > 2 {
                    self.received_bytes.extend_from_slice(&req[2..]);
                    self.transfer_offset += (req.len() - 2) as u32;
                }
                vec![0x76, seq]
            }
            // 0x37: RequestTransferExit
            0x37 => {
                if !self.download_active {
                    return vec![0x7F, 0x37, 0x24];
                }
                self.download_active = false;
                vec![0x77]
            }
            // 0x23: ReadMemoryByAddress (bench stock read)
            0x23 => {
                if req.len() < 3 {
                    return vec![0x7F, 0x23, 0x13];
                }
                let fmt = req[1];
                let addr_bytes = (fmt & 0x0F) as usize;
                let size_bytes = ((fmt >> 4) & 0x0F) as usize;
                let need = 2 + addr_bytes + size_bytes;
                if req.len() < need {
                    return vec![0x7F, 0x23, 0x13];
                }
                let mut addr = 0u64;
                for i in 0..addr_bytes {
                    addr = (addr << 8) | req[2 + i] as u64;
                }
                let mut count = 0u64;
                for i in 0..size_bytes {
                    count = (count << 8) | req[2 + addr_bytes + i] as u64;
                }
                if count == 0 || count > 4096 {
                    return vec![0x7F, 0x23, 0x31];
                }
                let mut resp = vec![0x63];
                for i in 0..count {
                    let b = virtual_stock_byte((addr + i) as u32);
                    resp.push(b);
                }
                resp
            }
            // 0x11: ECUReset
            0x11 => {
                self.unlocked_level1 = false;
                self.download_active = false;
                vec![0x51, if req.len() > 1 { req[1] } else { 0x01 }]
            }
            // 0x14: ClearDiagnosticInformation — intentionally unsupported (read-only compliance)
            0x14 => vec![0x7F, 0x14, 0x11],
            // 0x19: ReadDTCInformation (UDS)
            0x19 => {
                if req.len() < 2 {
                    return vec![0x7F, 0x19, 0x13];
                }
                match req[1] {
                    0x02 => {
                        // P0171 confirmed
                        vec![0x59, 0x02, 0xFF, 0x01, 0x01, 0x71, 0x00, 0x09]
                    }
                    0x04 => {
                        if req.len() >= 6 {
                            vec![
                                0x59, 0x04, req[2], req[3], req[5], 0x0C, 0x1A, 0x20, 0x05, 0x58, 0x0E,
                                0x8A,
                            ]
                        } else {
                            vec![0x7F, 0x19, 0x13]
                        }
                    }
                    _ => vec![0x7F, 0x19, 0x12],
                }
            }
            _ => vec![0x7F, req[0], 0x11], // Service not supported
        }
    }
}

fn virtual_stock_byte(address: u32) -> u8 {
    let a = address ^ 0xA5A5_0000;
    ((a >> 8) ^ (a & 0xFF) ^ 0x5A) as u8
}

impl PassThruDevice for VirtualEcuSimulator {
    fn open(&mut self) -> Result<(), String> {
        self.connected = true;
        self.start_time = Instant::now();
        Ok(())
    }

    fn close(&mut self) -> Result<(), String> {
        self.connected = false;
        self.rx_queue.clear();
        Ok(())
    }

    fn connect(&mut self, protocol: u32, _flags: u32, _baud: u32) -> Result<u32, String> {
        self.active_protocol = protocol;
        self.connected = true;
        Ok(self.channel_id)
    }

    fn disconnect(&mut self, _channel: u32) -> Result<(), String> {
        self.connected = false;
        Ok(())
    }

    fn read_msgs(&mut self, _channel: u32, _timeout_ms: u32) -> Result<Vec<PassThruMsg>, String> {
        if !self.connected {
            return Err("Simulator is disconnected".to_string());
        }
        let drained = self.rx_queue.drain(..).collect();
        Ok(drained)
    }

    fn write_msgs(&mut self, _channel: u32, msgs: &[PassThruMsg], _timeout_ms: u32) -> Result<u32, String> {
        if !self.connected {
            return Err("Simulator is disconnected".to_string());
        }
        for msg in msgs {
            let req_payload = msg.payload();
            if req_payload.is_empty() {
                continue;
            }
            if self.active_protocol == ISO14230 {
                if let Some(inner) = crate::ecu_bus::kwp::try_unpack_to_ecu(req_payload, 0x10) {
                    let resp_payload = self.handle_uds_request(&inner);
                    if let Ok(frame) = crate::ecu_bus::kwp::pack(0xF1, 0x10, &resp_payload) {
                        self.rx_queue.push(PassThruMsg::new(ISO14230, &frame));
                    }
                    continue;
                }
            } else if self.active_protocol == J1850VPW {
                if let Some(inner) = crate::ecu_bus::vpw::try_unpack_to_ecu(req_payload, 0x10) {
                    let resp_payload = self.handle_uds_request(&inner);
                    if let Ok(frame) = crate::ecu_bus::vpw::pack(0x6C, 0xF1, 0x10, &resp_payload) {
                        self.rx_queue.push(PassThruMsg::new(J1850VPW, &frame));
                    }
                    continue;
                }
            }
            let pci = req_payload[0] >> 4;
            let complete_req = if pci == 0x00 {
                let len = (req_payload[0] & 0x0F) as usize;
                if req_payload.len() >= 1 + len {
                    req_payload[1..1 + len].to_vec()
                } else {
                    req_payload[1..].to_vec()
                }
            } else if pci == 0x01 {
                self.rx_expected_len = (((req_payload[0] & 0x0F) as usize) << 8) | (req_payload[1] as usize);
                self.rx_assembly.clear();
                if req_payload.len() > 2 {
                    self.rx_assembly.extend_from_slice(&req_payload[2..]);
                }
                let tx_id = msg.read_id();
                let (rx_id, _) = bench_rx_id_for_tx(tx_id);
                let fc = vec![0x30, 0x00, 0x00, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA];
                let mut fc_msg = PassThruMsg::new_can(rx_id, &fc);
                fc_msg.protocol_id = self.active_protocol;
                self.rx_queue.push(fc_msg);
                continue;
            } else if pci == 0x02 {
                if req_payload.len() > 1 {
                    self.rx_assembly.extend_from_slice(&req_payload[1..]);
                }
                if self.rx_assembly.len() < self.rx_expected_len {
                    continue;
                }
                self.rx_assembly.truncate(self.rx_expected_len);
                self.rx_expected_len = 0;
                self.rx_assembly.clone()
            } else if pci == 0x03 {
                continue;
            } else {
                req_payload.to_vec()
            };

            let tx_id = msg.read_id();
            let (rx_id, role) = bench_rx_id_for_tx(tx_id);
            let resp_payload = self.handle_uds_request(&complete_req, role);

            if self.active_protocol == ISO15765 || self.active_protocol == CAN {
                self.enqueue_iso_tp_response(rx_id, &resp_payload);
            } else if self.active_protocol == ISO14230 {
                if let Ok(frame) = crate::ecu_bus::kwp::pack(0xF1, 0x10, &resp_payload) {
                    self.rx_queue.push(PassThruMsg::new(ISO14230, &frame));
                }
            } else if self.active_protocol == J1850VPW {
                if let Ok(frame) = crate::ecu_bus::vpw::pack(0x6C, 0xF1, 0x10, &resp_payload) {
                    self.rx_queue.push(PassThruMsg::new(J1850VPW, &frame));
                }
            } else {
                let mut resp_msg = PassThruMsg::default();
                resp_msg.protocol_id = self.active_protocol;
                let len = resp_payload.len().min(4128);
                resp_msg.data_size = len as u32;
                resp_msg.data[..len].copy_from_slice(&resp_payload[..len]);
                self.rx_queue.push(resp_msg);
            }
        }
        Ok(msgs.len() as u32)
    }

    fn start_filter(&mut self, _channel: u32, _filter_type: u32, _mask: &PassThruMsg, _pattern: &PassThruMsg, _flow_control: Option<&PassThruMsg>) -> Result<u32, String> {
        Ok(1)
    }

    fn ioctl(&mut self, _channel: u32, ioctl_id: u32, _input: &[u8]) -> Result<Vec<u8>, String> {
        match ioctl_id {
            READ_VBATT => {
                let elapsed = self.start_time.elapsed().as_secs_f32();
                let vbatt = 13.2 + (elapsed * 0.2).sin().abs() * 0.8;
                let mv = (vbatt * 1000.0) as u32;
                Ok(mv.to_le_bytes().to_vec())
            }
            FAST_INIT => Ok(vec![0xC1, 0xEA, 0x8F]), // KWP Key Bytes
            CLEAR_TX_BUFFER | CLEAR_RX_BUFFER => {
                self.rx_queue.clear();
                Ok(vec![])
            }
            _ => Ok(vec![]),
        }
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    fn firmware_version(&self) -> String {
        "Wideband-Sim 2.4.0 (Rust native virtual loopback)".to_string()
    }

    fn channel_id(&self) -> u32 {
        self.channel_id
    }

    fn device_label(&self) -> String {
        "virtual-ecu".to_string()
    }
}

#[cfg(windows)]
mod native_passthru {
    use super::*;
    use libloading::Library;
    use std::ffi::c_void;
    use std::ptr;

    type PassThruOpenFn = unsafe extern "system" fn(*const u16, *mut u32) -> i32;
    type PassThruCloseFn = unsafe extern "system" fn(u32) -> i32;
    type PassThruConnectFn = unsafe extern "system" fn(u32, u32, u32, u32, *mut u32) -> i32;
    type PassThruDisconnectFn = unsafe extern "system" fn(u32) -> i32;
    type PassThruReadFn = unsafe extern "system" fn(u32, *mut PassThruMsg, *mut u32, u32) -> i32;
    type PassThruWriteFn = unsafe extern "system" fn(u32, *mut PassThruMsg, *mut u32, u32) -> i32;
    type PassThruStartFilterFn =
        unsafe extern "system" fn(u32, u32, *mut PassThruMsg, *mut PassThruMsg, *mut PassThruMsg, *mut u32) -> i32;
    type PassThruIoctlFn = unsafe extern "system" fn(u32, u32, *mut c_void, *mut c_void) -> i32;

    pub struct NativeApi {
        library: Library,
        open: PassThruOpenFn,
        close: PassThruCloseFn,
        connect: PassThruConnectFn,
        disconnect: PassThruDisconnectFn,
        read: PassThruReadFn,
        write: PassThruWriteFn,
        start_filter: PassThruStartFilterFn,
        ioctl: PassThruIoctlFn,
    }

    impl NativeApi {
        pub fn load(path: &str) -> Result<Self, String> {
            let library = unsafe { Library::new(path) }.map_err(|e| format!("LoadLibrary failed: {}", e))?;
            unsafe {
                Ok(Self {
                    open: *library.get(b"PassThruOpen\0").map_err(|e| e.to_string())?,
                    close: *library.get(b"PassThruClose\0").map_err(|e| e.to_string())?,
                    connect: *library.get(b"PassThruConnect\0").map_err(|e| e.to_string())?,
                    disconnect: *library.get(b"PassThruDisconnect\0").map_err(|e| e.to_string())?,
                    read: *library.get(b"PassThruReadMsgs\0").map_err(|e| e.to_string())?,
                    write: *library.get(b"PassThruWriteMsgs\0").map_err(|e| e.to_string())?,
                    start_filter: *library.get(b"PassThruStartMsgFilter\0").map_err(|e| e.to_string())?,
                    ioctl: *library.get(b"PassThruIoctl\0").map_err(|e| e.to_string())?,
                    library,
                })
            }
        }

        pub fn open_device(&self) -> Result<u32, String> {
            let mut device_id = 0u32;
            let status = unsafe { (self.open)(ptr::null(), &mut device_id) };
            if status != STATUS_NOERROR {
                return Err(format!("PassThruOpen failed: {}", status));
            }
            Ok(device_id)
        }

        pub fn close_device(&self, device_id: u32) -> i32 {
            unsafe { (self.close)(device_id) }
        }

        pub fn disconnect_channel(&self, channel: u32) -> i32 {
            unsafe { (self.disconnect)(channel) }
        }

        pub fn connect_channel(
            &self,
            device_id: u32,
            protocol: u32,
            flags: u32,
            baud: u32,
            channel_id: &mut u32,
        ) -> i32 {
            unsafe { (self.connect)(device_id, protocol, flags, baud, channel_id) }
        }

        pub fn read_msgs(
            &self,
            channel: u32,
            msg: &mut PassThruMsg,
            count: &mut u32,
            timeout_ms: u32,
        ) -> i32 {
            unsafe { (self.read)(channel, msg, count, timeout_ms) }
        }

        pub fn write_msgs(
            &self,
            channel: u32,
            msg: &mut PassThruMsg,
            count: &mut u32,
            timeout_ms: u32,
        ) -> i32 {
            unsafe { (self.write)(channel, msg, count, timeout_ms) }
        }

        pub fn start_msg_filter(
            &self,
            channel: u32,
            filter_type: u32,
            mask: &mut PassThruMsg,
            pattern: &mut PassThruMsg,
            flow_control: *mut PassThruMsg,
            filter_id: &mut u32,
        ) -> i32 {
            unsafe {
                (self.start_filter)(channel, filter_type, mask, pattern, flow_control, filter_id)
            }
        }

        pub fn ioctl_raw(
            &self,
            channel: u32,
            ioctl_id: u32,
            input: *mut c_void,
            output: *mut c_void,
        ) -> i32 {
            unsafe { (self.ioctl)(channel, ioctl_id, input, output) }
        }
    }
}

pub struct NativePassThruDevice {
    pub lib_path: String,
    pub device_id: u32,
    pub channel_id: u32,
    pub connected: bool,
    pub version_str: String,
    #[cfg(windows)]
    api: Option<native_passthru::NativeApi>,
}

impl NativePassThruDevice {
    pub fn new(path: &str) -> Self {
        Self {
            lib_path: path.to_string(),
            device_id: 0,
            channel_id: 0,
            connected: false,
            version_str: "Unknown".to_string(),
            #[cfg(windows)]
            api: None,
        }
    }
}

impl PassThruDevice for NativePassThruDevice {
    fn open(&mut self) -> Result<(), String> {
        if !std::path::Path::new(&self.lib_path).exists() {
            return Err(format!("J2534 DLL not found: {}", self.lib_path));
        }
        #[cfg(windows)]
        {
            let api = native_passthru::NativeApi::load(&self.lib_path)?;
            self.device_id = api.open_device()?;
            self.api = Some(api);
            self.connected = true;
            self.version_str = format!("J2534 PassThru ({})", self.lib_path);
            return Ok(());
        }
        #[cfg(not(windows))]
        {
            Err("Native J2534 PassThru is only supported on Windows. Use Virtual ECU for bench.".to_string())
        }
    }

    fn close(&mut self) -> Result<(), String> {
        #[cfg(windows)]
        if let Some(api) = &self.api {
            if self.channel_id != 0 {
                let _ = api.disconnect_channel(self.channel_id);
                self.channel_id = 0;
            }
            if self.device_id != 0 {
                let _ = api.close_device(self.device_id);
                self.device_id = 0;
            }
        }
        self.connected = false;
        #[cfg(windows)]
        {
            self.api = None;
        }
        Ok(())
    }

    fn connect(&mut self, protocol: u32, flags: u32, baud: u32) -> Result<u32, String> {
        #[cfg(windows)]
        {
            let api = self.api.as_ref().ok_or("PassThru not open")?;
            let mut channel_id = 0u32;
            let status = api.connect_channel(self.device_id, protocol, flags, baud, &mut channel_id);
            if status != STATUS_NOERROR {
                return Err(format!("PassThruConnect failed: {}", status));
            }
            self.channel_id = channel_id;
            self.connected = true;
            return Ok(channel_id);
        }
        #[cfg(not(windows))]
        {
            Err("Native J2534 connect requires Windows".to_string())
        }
    }

    fn disconnect(&mut self, channel: u32) -> Result<(), String> {
        #[cfg(windows)]
        if let Some(api) = &self.api {
            let _ = api.disconnect_channel(channel);
        }
        if self.channel_id == channel {
            self.channel_id = 0;
        }
        Ok(())
    }

    fn read_msgs(&mut self, channel: u32, timeout_ms: u32) -> Result<Vec<PassThruMsg>, String> {
        #[cfg(windows)]
        {
            let api = self.api.as_ref().ok_or("PassThru not open")?;
            let mut msg = PassThruMsg::default();
            let mut count: u32 = 1;
            let status = api.read_msgs(channel, &mut msg, &mut count, timeout_ms);
            if status == 0x09 || status == 0x10 {
                return Ok(Vec::new());
            }
            if status != STATUS_NOERROR {
                return Err(format!("PassThruReadMsgs failed: status 0x{:X}", status as u32));
            }
            if count == 0 {
                return Ok(Vec::new());
            }
            return Ok(vec![msg]);
        }
        #[cfg(not(windows))]
        {
            Ok(Vec::new())
        }
    }

    fn write_msgs(&mut self, channel: u32, msgs: &[PassThruMsg], timeout_ms: u32) -> Result<u32, String> {
        #[cfg(windows)]
        {
            let api = self.api.as_ref().ok_or("PassThru not open")?;
            let mut written = 0u32;
            for msg in msgs {
                let mut one = *msg;
                let mut count = 1u32;
                let status = api.write_msgs(channel, &mut one, &mut count, timeout_ms);
                if status != STATUS_NOERROR {
                    return Err(format!("PassThruWriteMsgs failed: {}", status));
                }
                written += count;
            }
            return Ok(written);
        }
        #[cfg(not(windows))]
        {
            Ok(msgs.len() as u32)
        }
    }

    fn start_filter(
        &mut self,
        channel: u32,
        filter_type: u32,
        mask: &PassThruMsg,
        pattern: &PassThruMsg,
        flow_control: Option<&PassThruMsg>,
    ) -> Result<u32, String> {
        #[cfg(windows)]
        {
            let api = self.api.as_ref().ok_or("PassThru not open")?;
            let mut mask_msg = *mask;
            let mut pattern_msg = *pattern;
            let mut flow_msg = flow_control.copied().unwrap_or_default();
            let flow_ptr = if flow_control.is_some() {
                &mut flow_msg as *mut PassThruMsg
            } else {
                std::ptr::null_mut()
            };
            let mut filter_id = 0u32;
            let status = api.start_msg_filter(
                channel,
                filter_type,
                &mut mask_msg,
                &mut pattern_msg,
                flow_ptr,
                &mut filter_id,
            );
            if status != STATUS_NOERROR {
                return Err(format!("PassThruStartMsgFilter failed: {}", status));
            }
            Ok(filter_id)
        }
        #[cfg(not(windows))]
        {
            Ok(1)
        }
    }

    fn ioctl(&mut self, channel: u32, ioctl_id: u32, input: &[u8]) -> Result<Vec<u8>, String> {
        #[cfg(windows)]
        {
            let api = self.api.as_ref().ok_or("PassThru not open")?;
            let mut in_buf = input.to_vec();
            let mut out_buf = vec![0u8; 256];
            let status = api.ioctl_raw(
                channel,
                ioctl_id,
                in_buf.as_mut_ptr() as *mut c_void,
                out_buf.as_mut_ptr() as *mut c_void,
            );
            if status != STATUS_NOERROR {
                return Ok(vec![]);
            }
            return Ok(out_buf);
        }
        #[cfg(not(windows))]
        {
            Ok(vec![])
        }
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    fn firmware_version(&self) -> String {
        self.version_str.clone()
    }

    fn channel_id(&self) -> u32 {
        if self.channel_id != 0 {
            self.channel_id
        } else {
            1
        }
    }

    fn device_label(&self) -> String {
        self.lib_path.clone()
    }
}

/// Connect J2534 channel and install ISO-15765 filters when required.
pub fn connect_for_profile(
    device: &mut dyn PassThruDevice,
    profile: &crate::protocols::EcuProfile,
) -> Result<u32, String> {
    use crate::protocols::VehicleBus;
    device.set_connect_hints(profile.extended_id, profile.response_id, profile.filter_mask);
    let channel = device.connect(profile.protocol_id, profile.connect_flags, profile.bitrate)?;
    if profile.bus == VehicleBus::CanIsoTp {
        let mut pass_mask = PassThruMsg::new_can(profile.filter_mask, &[]);
        pass_mask.write_id(profile.filter_mask);
        let mut pass_pat = PassThruMsg::new_can(profile.response_id, &[]);
        pass_pat.write_id(profile.response_id);
        device.start_filter(channel, PASS_FILTER, &pass_mask, &pass_pat, None)?;
    } else if profile.bus == VehicleBus::KLine {
        let wake = crate::ecu_bus::kwp::pack(profile.ecu, profile.tester, &[0x81])?;
        device.kline_fast_init(channel, &wake)?;
    }
    Ok(channel)
}

pub fn create_device(info: &J2534DeviceInfo) -> Box<dyn PassThruDevice> {
    if info.is_virtual || info.library_path == "virtual" {
        Box::new(VirtualEcuSimulator::new())
    } else {
        #[cfg(windows)]
        {
            if crate::j2534_host::HostPassThruDevice::host_available() {
                return crate::j2534_host::create_host_device(&info.library_path);
            }
        }
        Box::new(NativePassThruDevice::new(&info.library_path))
    }
}

/// Human-readable transport hint for logs (host vs in-process DLL).
pub fn transport_hint_for_device(info: &J2534DeviceInfo) -> &'static str {
    if info.is_virtual {
        return "virtual loopback";
    }
    #[cfg(windows)]
    {
        if crate::j2534_host::HostPassThruDevice::host_available() {
            return "J2534Host.exe (32-bit DLL bridge)";
        }
    }
    "in-process PassThru (requires matching DLL bitness)"
}
