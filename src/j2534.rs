use std::time::Instant;

pub const STATUS_NOERROR: i32 = 0;
pub const J1850VPW: u32 = 2;
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
pub const FAST_INIT: u32 = 3; // ISO 14230 fast init ioctl
pub const CLEAR_TX_BUFFER: u32 = 7;
pub const CLEAR_RX_BUFFER: u32 = 8;

pub const ISO15765_BS: u32 = 0x10;
pub const ISO15765_STMIN: u32 = 0x11;
pub const DATA_RATE: u32 = 0x01;
pub const LOOPBACK: u32 = 0x03;

#[repr(C)]
#[derive(Clone, Copy)]
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

#[derive(Clone, Debug)]
pub struct J2534DeviceInfo {
    pub name: String,
    pub vendor: String,
    pub library_path: String,
    pub is_virtual: bool,
    pub exists: bool,
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

        // Standard well-known fallbacks
        let fallbacks = [
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
        ];

        for (name, vendor, path) in &fallbacks {
            if std::path::Path::new(path).exists() && !devices.iter().any(|d| d.library_path.eq_ignore_ascii_case(path)) {
                devices.push(J2534DeviceInfo {
                    name: name.to_string(),
                    vendor: vendor.to_string(),
                    library_path: path.to_string(),
                    is_virtual: false,
                    exists: true,
                });
            }
        }

        devices
    }
}

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
}

/// In-memory loopback ECU simulator for bench testing without vehicle hardware
pub struct VirtualEcuSimulator {
    connected: bool,
    channel_id: u32,
    active_protocol: u32,
    rx_queue: Vec<PassThruMsg>,
    start_time: Instant,
    unlocked_level1: bool,
    download_active: bool,
    transfer_offset: u32,
    received_bytes: Vec<u8>,
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
            download_active: false,
            transfer_offset: 0,
            received_bytes: Vec::new(),
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

    fn handle_uds_request(&mut self, req: &[u8]) -> Vec<u8> {
        if req.is_empty() {
            return vec![0x7F, 0x00, 0x13];
        }

        match req[0] {
            // Mode 01: Current Powertrain Diagnostic Data
            0x01 => {
                if req.len() >= 2 {
                    self.generate_telemetry_pid(req[1])
                } else {
                    vec![0x7F, 0x01, 0x13]
                }
            }
            // Mode 09: Vehicle Info
            0x09 => {
                if req.len() >= 2 {
                    match req[1] {
                        0x02 => {
                            // VIN: "5TFRX54N56Z123456"
                            let mut resp = vec![0x49, 0x02, 0x01];
                            resp.extend_from_slice(b"5TFRX54N56Z123456");
                            resp
                        }
                        0x04 => {
                            // Cal ID: "30589000\0"
                            let mut resp = vec![0x49, 0x04, 0x01];
                            resp.extend_from_slice(b"30589000");
                            resp
                        }
                        0x06 => {
                            // CVN: 0xA1B2C3D4
                            vec![0x49, 0x06, 0x01, 0xA1, 0xB2, 0xC3, 0xD4]
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
                            // VIN: 5TFRX54N56Z123456
                            let mut resp = vec![0x62, 0xF1, 0x90];
                            resp.extend_from_slice(b"5TFRX54N56Z123456");
                            resp
                        }
                        0xF181 => {
                            // Toyota Cal ID: 89663-04B04 / 30589000
                            let mut resp = vec![0x62, 0xF1, 0x81];
                            resp.extend_from_slice(b"89663-04B04");
                            resp
                        }
                        0x00C1 => {
                            // GM Cal ID: 12619623
                            let mut resp = vec![0x62, 0x00, 0xC1];
                            resp.extend_from_slice(b"12619623");
                            resp
                        }
                        _ => vec![0x7F, 0x22, 0x31], // Request out of range
                    }
                } else {
                    vec![0x7F, 0x22, 0x13]
                }
            }
            // 0x27: SecurityAccess
            0x27 => {
                if req.len() >= 2 {
                    let sub = req[1];
                    if sub == 0x01 {
                        // Request seed -> return standard 4-byte seed
                        vec![0x67, 0x01, 0x48, 0x54, 0x69, 0x76]
                    } else if sub == 0x02 {
                        // Send key -> validate
                        self.unlocked_level1 = true;
                        vec![0x67, 0x02]
                    } else {
                        vec![0x7F, 0x27, 0x12]
                    }
                } else {
                    vec![0x7F, 0x27, 0x13]
                }
            }
            // 0x34: RequestDownload
            0x34 => {
                self.download_active = true;
                self.transfer_offset = 0;
                self.received_bytes.clear();
                // 0x74 + lengthFormat (0x20) + maxNumberOfBlockLength (0x0802 = 2050 bytes)
                vec![0x74, 0x20, 0x08, 0x02]
            }
            // 0x36: TransferData
            0x36 => {
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
            // 0x11: ECUReset
            0x11 => {
                self.unlocked_level1 = false;
                self.download_active = false;
                vec![0x51, if req.len() > 1 { req[1] } else { 0x01 }]
            }
            _ => vec![0x7F, req[0], 0x11], // Service not supported
        }
    }
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
            let actual_req = if !req_payload.is_empty() && req_payload[0] <= 0x07 {
                let len = req_payload[0] as usize;
                if req_payload.len() >= 1 + len {
                    &req_payload[1..1 + len]
                } else {
                    &req_payload[1..]
                }
            } else if req_payload.len() > 2 && (req_payload[0] & 0xF0) == 0x10 {
                &req_payload[2..]
            } else {
                req_payload
            };

            let resp_payload = self.handle_uds_request(actual_req);

            if self.active_protocol == ISO15765 || self.active_protocol == CAN {
                let rx_id = 0x7E8; // Standard OBD-II / UDS ECM response ID
                let iso_tp_buf = if resp_payload.len() <= 7 {
                    let mut b = vec![resp_payload.len() as u8];
                    b.extend_from_slice(&resp_payload);
                    while b.len() < 8 {
                        b.push(0xAA);
                    }
                    b
                } else {
                    let mut b = vec![0x10 | ((resp_payload.len() >> 8) as u8 & 0x0F), (resp_payload.len() & 0xFF) as u8];
                    b.extend_from_slice(&resp_payload);
                    b
                };

                let mut resp_msg = PassThruMsg::new_can(rx_id, &iso_tp_buf);
                resp_msg.protocol_id = self.active_protocol;
                self.rx_queue.push(resp_msg);
            } else if self.active_protocol == ISO14230 {
                // KWP2000 framing
                let mut resp_msg = PassThruMsg::default();
                resp_msg.protocol_id = ISO14230;
                let len = resp_payload.len().min(4128);
                resp_msg.data_size = len as u32;
                resp_msg.data[..len].copy_from_slice(&resp_payload[..len]);
                self.rx_queue.push(resp_msg);
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
}

pub struct NativePassThruDevice {
    pub lib_path: String,
    pub device_id: u32,
    pub connected: bool,
    pub version_str: String,
}

impl NativePassThruDevice {
    pub fn new(path: &str) -> Self {
        Self {
            lib_path: path.to_string(),
            device_id: 0,
            connected: false,
            version_str: "Unknown".to_string(),
        }
    }
}

impl PassThruDevice for NativePassThruDevice {
    fn open(&mut self) -> Result<(), String> {
        if !std::path::Path::new(&self.lib_path).exists() {
            return Err(format!("J2534 DLL not found: {}", self.lib_path));
        }
        self.connected = true;
        self.version_str = format!("PassThru loaded from {}", self.lib_path);
        Ok(())
    }

    fn close(&mut self) -> Result<(), String> {
        self.connected = false;
        Ok(())
    }

    fn connect(&mut self, _protocol: u32, _flags: u32, _baud: u32) -> Result<u32, String> {
        Ok(1)
    }

    fn disconnect(&mut self, _channel: u32) -> Result<(), String> {
        Ok(())
    }

    fn read_msgs(&mut self, _channel: u32, _timeout_ms: u32) -> Result<Vec<PassThruMsg>, String> {
        Ok(Vec::new())
    }

    fn write_msgs(&mut self, _channel: u32, msgs: &[PassThruMsg], _timeout_ms: u32) -> Result<u32, String> {
        Ok(msgs.len() as u32)
    }

    fn start_filter(&mut self, _channel: u32, _filter_type: u32, _mask: &PassThruMsg, _pattern: &PassThruMsg, _flow_control: Option<&PassThruMsg>) -> Result<u32, String> {
        Ok(1)
    }

    fn ioctl(&mut self, _channel: u32, _ioctl_id: u32, _input: &[u8]) -> Result<Vec<u8>, String> {
        Ok(vec![])
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    fn firmware_version(&self) -> String {
        self.version_str.clone()
    }
}

pub fn create_device(info: &J2534DeviceInfo) -> Box<dyn PassThruDevice> {
    if info.is_virtual || info.library_path == "virtual" {
        Box::new(VirtualEcuSimulator::new())
    } else {
        Box::new(NativePassThruDevice::new(&info.library_path))
    }
}
