//! 32-bit `J2534Host.exe` IPC bridge (same line protocol as C# `CanLink`).
//! Used when the Rust binary is 64-bit and cannot load x86 PassThru DLLs directly.

#[cfg(windows)]
mod imp {
    use super::super::j2534::{PassThruDevice, PassThruMsg, CAN, ISO15765};
    use serde_json::Value;
    use std::io::{BufRead, BufReader, Write};
    use std::path::{Path, PathBuf};
    use std::process::{Child, ChildStdin, Command, Stdio};
    use std::sync::Mutex;

    pub struct HostPassThruDevice {
        lib_path: String,
        host_path: PathBuf,
        child: Option<Child>,
        stdin: Option<ChildStdin>,
        reader: Mutex<Option<BufReader<std::process::ChildStdout>>>,
        device_id: u32,
        channel_id: u32,
        connected: bool,
        extended: bool,
        version_str: String,
        connect_extended: bool,
        connect_filter_id: u32,
        connect_filter_mask: u32,
        active_protocol: u32,
    }

    impl HostPassThruDevice {
        pub fn new(dll_path: &str) -> Self {
            Self {
                lib_path: dll_path.to_string(),
                host_path: Self::locate_host_exe(),
                child: None,
                stdin: None,
                reader: Mutex::new(None),
                device_id: 0,
                channel_id: 0,
                connected: false,
                extended: false,
                version_str: "J2534Host".to_string(),
                connect_extended: false,
                connect_filter_id: 0x7E8,
                connect_filter_mask: 0x7FF,
                active_protocol: 5,
            }
        }

        pub fn host_available() -> bool {
            Self::locate_host_exe().exists()
        }

        fn locate_host_exe() -> PathBuf {
            if let Ok(exe) = std::env::current_exe() {
                let beside = exe.parent().unwrap_or(Path::new(".")).join("J2534Host.exe");
                if beside.exists() {
                    return beside;
                }
            }
            PathBuf::from("J2534Host.exe")
        }

        fn command(&mut self, line: &str, timeout_ms: u32) -> Result<Value, String> {
            let stdin = self.stdin.as_mut().ok_or("J2534Host stdin not open")?;
            writeln!(stdin, "{}", line).map_err(|e| format!("host write failed: {}", e))?;
            stdin.flush().map_err(|e| e.to_string())?;

            let mut guard = self.reader.lock().map_err(|_| "host reader lock")?;
            let reader = guard.as_mut().ok_or("J2534Host stdout not open")?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms as u64);
            while std::time::Instant::now() < deadline {
                let mut line_buf = String::new();
                reader.read_line(&mut line_buf).map_err(|e| format!("host read failed: {}", e))?;
                if let Some(start) = line_buf.find('{') {
                    if let Some(end) = line_buf.rfind('}') {
                        let json = &line_buf[start..=end];
                        return Ok(serde_json::from_str(json).map_err(|e| e.to_string())?);
                    }
                }
            }
            Err("J2534Host response timed out".to_string())
        }

        fn require_ok(v: &Value) -> Result<(), String> {
            if v.get("ok").and_then(|b| b.as_bool()) == Some(true) {
                return Ok(());
            }
            let err = v.get("error").and_then(|e| e.as_str()).unwrap_or("J2534Host error");
            if let Some(st) = v.get("status").and_then(|s| s.as_i64()) {
                if st >= 0 {
                    return Err(format!("{} (status 0x{:X})", err, st as u32));
                }
            }
            Err(err.to_string())
        }

        fn is_tx_full(v: &Value) -> bool {
            v.get("full").and_then(|b| b.as_bool()) == Some(true)
        }

        fn is_can_protocol(&self) -> bool {
            self.active_protocol == ISO15765 || self.active_protocol == CAN
        }
    }

    impl PassThruDevice for HostPassThruDevice {
        fn set_connect_hints(&mut self, extended: bool, filter_id: u32, filter_mask: u32) {
            self.connect_extended = extended;
            self.connect_filter_id = filter_id;
            self.connect_filter_mask = filter_mask;
        }

        fn open(&mut self) -> Result<(), String> {
            if !self.host_path.exists() {
                return Err(format!("J2534Host.exe not found (expected {})", self.host_path.display()));
            }
            if !Path::new(&self.lib_path).exists() {
                return Err(format!("J2534 DLL not found: {}", self.lib_path));
            }
            let mut child = Command::new(&self.host_path)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| format!("failed to start J2534Host: {}", e))?;
            let stdin = child.stdin.take().ok_or("host stdin missing")?;
            let stdout = child.stdout.take().ok_or("host stdout missing")?;
            self.child = Some(child);
            self.stdin = Some(stdin);
            *self.reader.lock().unwrap() = Some(BufReader::new(stdout));

            let quoted = if self.lib_path.contains(' ') {
                format!("\"{}\"", self.lib_path)
            } else {
                self.lib_path.clone()
            };
            let resp = self.command(&format!("OPEN {}", quoted), 15_000)?;
            Self::require_ok(&resp)?;
            self.device_id = resp.get("device_id").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            self.connected = true;
            if let Ok(ver) = self.command("VERSION", 15_000) {
                if ver.get("ok").and_then(|b| b.as_bool()) == Some(true) {
                    let fw = ver.get("firmware").and_then(|v| v.as_str()).unwrap_or("?");
                    let dll = ver.get("dll").and_then(|v| v.as_str()).unwrap_or("?");
                    let api = ver.get("api").and_then(|v| v.as_str()).unwrap_or("?");
                    self.version_str = format!("J2534Host FW:{} DLL:{} API:{}", fw, dll, api);
                } else {
                    self.version_str = format!("J2534Host → {}", self.lib_path);
                }
            } else {
                self.version_str = format!("J2534Host → {}", self.lib_path);
            }
            Ok(())
        }

        fn close(&mut self) -> Result<(), String> {
            if self.stdin.is_some() {
                let _ = self.command("CLOSE", 3000);
                let _ = self.command("QUIT", 1000);
            }
            if let Some(mut child) = self.child.take() {
                let _ = child.kill();
            }
            self.stdin = None;
            *self.reader.lock().unwrap() = None;
            self.connected = false;
            self.channel_id = 0;
            Ok(())
        }

        fn connect(&mut self, protocol: u32, flags: u32, baud: u32) -> Result<u32, String> {
            let ext_flag = if self.connect_extended { 1 } else { 0 };
            let line = format!(
                "CONNECT {} {} {} {:X}",
                baud,
                ext_flag,
                protocol,
                flags
            );
            let resp = self.command(&line, 15_000)?;
            Self::require_ok(&resp)?;
            self.channel_id = resp.get("channel_id").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            self.active_protocol = protocol;
            self.extended = self.connect_extended && protocol == 5;
            if protocol == 5 {
                let filter_line = format!(
                    "FILTER {:X} {:X}",
                    self.connect_filter_id,
                    self.connect_filter_mask
                );
                let f = self.command(&filter_line, 15_000)?;
                Self::require_ok(&f)?;
            } else {
                let f = self.command("FILTER 00 00", 15_000)?;
                Self::require_ok(&f)?;
            }
            Ok(self.channel_id)
        }

        fn disconnect(&mut self, _channel: u32) -> Result<(), String> {
            Ok(())
        }

        fn read_msgs(&mut self, _channel: u32, timeout_ms: u32) -> Result<Vec<PassThruMsg>, String> {
            let resp = self.command("READ", timeout_ms.max(50))?;
            Self::require_ok(&resp)?;
            if resp.get("empty").and_then(|v| v.as_bool()) == Some(true) {
                return Ok(Vec::new());
            }
            let data_hex = resp.get("data").and_then(|v| v.as_str()).unwrap_or("");
            let payload = hex_to_bytes(data_hex);
            if !self.is_can_protocol() {
                return Ok(vec![PassThruMsg::new(self.active_protocol, &payload)]);
            }
            let id_hex = resp.get("id").and_then(|v| v.as_str()).unwrap_or("0");
            let id = u32::from_str_radix(id_hex, 16).unwrap_or(0);
            let mut msg = PassThruMsg::new_can(id, &[]);
            msg.write_id(id);
            let len = payload.len().min(4124);
            msg.data_size = (4 + len) as u32;
            msg.data[4..4 + len].copy_from_slice(&payload[..len]);
            Ok(vec![msg])
        }

        fn write_msgs(&mut self, _channel: u32, msgs: &[PassThruMsg], timeout_ms: u32) -> Result<u32, String> {
            let mut count = 0u32;
            for msg in msgs {
                let line = if self.is_can_protocol() {
                    let id = msg.read_id();
                    let payload = msg.payload();
                    format!("WRITE {:X} {}", id, bytes_to_hex(payload))
                } else {
                    let raw = if msg.protocol_id == ISO15765 || msg.protocol_id == CAN {
                        msg.payload()
                    } else {
                        &msg.data[..msg.data_size as usize]
                    };
                    format!("WRITE {}", bytes_to_hex(raw))
                };
                let mut attempts = 0u32;
                while attempts < 8 {
                    let resp = self.command(&line, timeout_ms.max(1000))?;
                    if Self::is_tx_full(&resp) {
                        attempts += 1;
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        continue;
                    }
                    Self::require_ok(&resp)?;
                    count += 1;
                    break;
                }
                if attempts >= 8 {
                    return Err("PassThru TX buffer full after retries (cable busy)".to_string());
                }
            }
            Ok(count)
        }

        fn start_filter(
            &mut self,
            _channel: u32,
            _filter_type: u32,
            _mask: &PassThruMsg,
            _pattern: &PassThruMsg,
            _flow_control: Option<&PassThruMsg>,
        ) -> Result<u32, String> {
            Ok(1)
        }

        fn ioctl(&mut self, _channel: u32, _ioctl_id: u32, _input: &[u8]) -> Result<Vec<u8>, String> {
            Ok(vec![])
        }

        fn kline_fast_init(&mut self, _channel: u32, frame: &[u8]) -> Result<Vec<u8>, String> {
            let line = format!("INIT {}", bytes_to_hex(frame));
            let resp = self.command(&line, 15_000)?;
            Self::require_ok(&resp)?;
            let data_hex = resp.get("data").and_then(|v| v.as_str()).unwrap_or("");
            Ok(hex_to_bytes(data_hex))
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
            format!("j2534-host:{}", self.lib_path)
        }
    }

    pub fn create_host_device(dll_path: &str) -> Box<dyn PassThruDevice> {
        Box::new(HostPassThruDevice::new(dll_path))
    }

    fn hex_to_bytes(hex: &str) -> Vec<u8> {
        let s = hex.trim();
        if s.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut i = 0;
        while i + 1 < s.len() {
            if let Ok(b) = u8::from_str_radix(&s[i..i + 2], 16) {
                out.push(b);
            }
            i += 2;
        }
        out
    }

    fn bytes_to_hex(data: &[u8]) -> String {
        data.iter().map(|b| format!("{:02X}", b)).collect()
    }
}

#[cfg(windows)]
pub use imp::{create_host_device, HostPassThruDevice};

#[cfg(not(windows))]
pub fn create_host_device(_dll_path: &str) -> Box<dyn crate::j2534::PassThruDevice> {
    Box::new(crate::j2534::NativePassThruDevice::new(""))
}

#[cfg(not(windows))]
pub struct HostPassThruDevice;

#[cfg(not(windows))]
impl HostPassThruDevice {
    pub fn host_available() -> bool {
        false
    }
}
