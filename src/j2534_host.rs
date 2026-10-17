//! 32-bit `J2534Host.exe` IPC bridge (same line protocol as C# `CanLink`).
//! Used when the Rust binary is 64-bit and cannot load x86 PassThru DLLs directly.

#[cfg(windows)]
mod imp {
    use super::super::j2534::{PassThruDevice, PassThruMsg, CAN, ISO15765};
    use serde_json::Value;
    use std::io::{BufRead, BufReader, Write};
    use std::path::{Path, PathBuf};
    use std::process::{Child, ChildStdin, Command, Stdio};
    use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    /// Floor for any host reply. A raw CAN READ waits at most its own timeout in the DLL, but the
    /// XHorse Mini-VCI driver blocks about 5 s on an empty ISO15765 channel whatever timeout it is
    /// given, so the floor sits above that. Hitting it means the host or vendor DLL is wedged.
    const MIN_HOST_REPLY_MS: u32 = 8000;
    /// Longest wait handed to the DLL for one READ; the ISO-TP and K-Line loops poll again.
    const MAX_READ_WAIT_MS: u32 = 50;
    /// SAE J2534-1 protocol id for ISO15765 on the host wire (raw CAN is 5).
    const WIRE_ISO15765: u32 = 6;

    pub struct HostPassThruDevice {
        lib_path: String,
        host_path: PathBuf,
        child: Option<Child>,
        stdin: Option<ChildStdin>,
        /// Lines from the host's stdout, fed by a reader thread so replies can time out.
        lines: Mutex<Option<Receiver<String>>>,
        device_id: u32,
        channel_id: u32,
        connected: bool,
        extended: bool,
        version_str: String,
        connect_extended: bool,
        connect_filter_id: u32,
        connect_filter_mask: u32,
        connect_request_id: u32,
        active_protocol: u32,
        /// The channel is J2534 ISO15765 (protocol 6) and the cable runs ISO-TP.
        cable_isotp: bool,
        /// `j2534-trace.log` beside the program: every host command, reply, and its duration.
        trace: Option<std::fs::File>,
        trace_lines: u32,
    }

    impl HostPassThruDevice {
        pub fn new(dll_path: &str) -> Self {
            Self {
                lib_path: dll_path.to_string(),
                host_path: Self::locate_host_exe(),
                child: None,
                stdin: None,
                lines: Mutex::new(None),
                device_id: 0,
                channel_id: 0,
                connected: false,
                extended: false,
                version_str: "J2534Host".to_string(),
                connect_extended: false,
                connect_filter_id: 0x7E8,
                connect_filter_mask: 0x7FF,
                connect_request_id: 0x7E0,
                active_protocol: 5,
                cable_isotp: false,
                trace: None,
                trace_lines: 0,
            }
        }

        pub fn host_available() -> bool {
            Self::locate_host_exe().exists()
        }

        /// Actionable copy for connect / first-run when the x86 host is absent on 64-bit Windows.
        pub fn missing_host_user_message() -> String {
            let path = Self::locate_host_exe();
            format!(
                "J2534Host.exe is missing (expected {}). On Windows, run scripts\\Package-WindowsRelease.ps1 or compile J2534Host.cs with csc.exe /platform:x86, then run scripts\\Friday-CablePrep.ps1 before connect.",
                path.display()
            )
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

        /// Starts a fresh trace for this cable session; tracing is best effort and never fails a call.
        fn open_trace(&mut self) {
            let dir = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.to_path_buf()))
                .unwrap_or_else(|| PathBuf::from("."));
            self.trace = std::fs::File::create(dir.join("j2534-trace.log")).ok();
            self.trace_lines = 0;
            let started = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
            let note = format!("Wideband J2534 trace {} — {}", started, self.lib_path);
            self.trace_line(&note);
        }

        fn trace_line(&mut self, text: &str) {
            const MAX_TRACE_LINES: u32 = 50_000;
            if let Some(file) = self.trace.as_mut() {
                if self.trace_lines >= MAX_TRACE_LINES {
                    return;
                }
                self.trace_lines += 1;
                let stamp = chrono::Local::now().format("%H:%M:%S%.3f");
                let _ = writeln!(file, "{} {}", stamp, text);
                if self.trace_lines == MAX_TRACE_LINES {
                    let _ = writeln!(file, "{} trace limit reached", stamp);
                }
                let _ = file.flush();
            }
        }

        fn command(&mut self, line: &str, timeout_ms: u32) -> Result<Value, String> {
            let sent = Instant::now();
            self.trace_line(&format!(">> {}", line));
            let result = self.command_untraced(line, timeout_ms);
            let ms = sent.elapsed().as_millis();
            match &result {
                Ok(v) => self.trace_line(&format!("<< {} ({} ms)", v, ms)),
                Err(e) => self.trace_line(&format!("!! {} ({} ms)", e, ms)),
            }
            result
        }

        fn command_untraced(&mut self, line: &str, timeout_ms: u32) -> Result<Value, String> {
            let stdin = self.stdin.as_mut().ok_or("J2534Host stdin not open")?;
            writeln!(stdin, "{}", line).map_err(|e| format!("host write failed: {}", e))?;
            stdin.flush().map_err(|e| e.to_string())?;

            let wait = Duration::from_millis(timeout_ms.max(MIN_HOST_REPLY_MS) as u64);
            let deadline = Instant::now() + wait;
            let outcome = {
                let guard = self.lines.lock().map_err(|_| "host reader lock")?;
                let rx = guard.as_ref().ok_or("J2534Host stdout not open")?;
                loop {
                    let left = deadline.saturating_duration_since(Instant::now());
                    match rx.recv_timeout(left) {
                        Ok(line_buf) => {
                            if let (Some(start), Some(end)) = (line_buf.find('{'), line_buf.rfind('}')) {
                                if start <= end {
                                    break Ok(line_buf[start..=end].to_string());
                                }
                            }
                        }
                        Err(RecvTimeoutError::Timeout) => {
                            break Err(format!(
                                "J2534Host did not answer within {} ms (host or PassThru DLL hung); session closed",
                                wait.as_millis()
                            ))
                        }
                        Err(RecvTimeoutError::Disconnected) => {
                            break Err("J2534Host exited unexpectedly; session closed".to_string())
                        }
                    }
                }
            };
            match outcome {
                Ok(json) => serde_json::from_str(&json).map_err(|e| e.to_string()),
                Err(e) => {
                    // A late reply would be read as the answer to the next command; drop the host.
                    self.kill_host();
                    Err(e)
                }
            }
        }

        fn kill_host(&mut self) {
            if let Some(mut child) = self.child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
            self.stdin = None;
            if let Ok(mut g) = self.lines.lock() {
                *g = None;
            }
            self.connected = false;
            self.channel_id = 0;
        }

        /// Opens a J2534 ISO15765 channel with a flow-control filter, as Techstream does.
        fn connect_cable_isotp(&mut self, flags: u32, baud: u32) -> Result<u32, String> {
            let ext_flag = if self.connect_extended { 1 } else { 0 };
            let line = format!("CONNECT {} {} {} {:X}", baud, ext_flag, WIRE_ISO15765, flags);
            let resp = self.command(&line, 15_000)?;
            Self::require_ok(&resp)?;
            let channel = resp.get("channel_id").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            // A flow-control filter pairs one exact response id with one request id.
            let exact = if self.connect_extended { 0x1FFF_FFFF } else { 0x7FF };
            let filter_line = format!(
                "FILTER {:X} {:X} {:X}",
                self.connect_filter_id, exact, self.connect_request_id
            );
            let f = self.command(&filter_line, 15_000)?;
            Self::require_ok(&f)?;
            Ok(channel)
        }

        fn require_ok(v: &Value) -> Result<(), String> {
            if v.get("ok").and_then(|b| b.as_bool()) == Some(true) {
                return Ok(());
            }
            let err = v.get("error").and_then(|e| e.as_str()).unwrap_or("J2534Host error");
            if let Some(st) = v.get("status").and_then(|s| s.as_i64()) {
                if st >= 0 {
                    return Err(crate::j2534::with_status_hint(
                        format!("{} (status 0x{:X})", err, st as u32),
                        st as u32,
                    ));
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

        fn set_flow_control_id(&mut self, request_id: u32) {
            self.connect_request_id = request_id;
        }

        fn cable_isotp(&self) -> bool {
            self.cable_isotp
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
            self.open_trace();
            self.stdin = Some(stdin);
            let (tx, rx) = mpsc::channel::<String>();
            std::thread::Builder::new()
                .name("j2534host-stdout".into())
                .spawn(move || {
                    let mut reader = BufReader::new(stdout);
                    loop {
                        let mut line = String::new();
                        match reader.read_line(&mut line) {
                            Ok(0) | Err(_) => break,
                            Ok(_) => {
                                if tx.send(line).is_err() {
                                    break;
                                }
                            }
                        }
                    }
                })
                .map_err(|e| format!("failed to start host reader: {}", e))?;
            *self.lines.lock().unwrap() = Some(rx);

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
                // QUIT gets no reply (host just exits), so send it without waiting.
                if let Some(stdin) = self.stdin.as_mut() {
                    let _ = writeln!(stdin, "QUIT");
                    let _ = stdin.flush();
                }
            }
            self.kill_host();
            Ok(())
        }

        fn connect(&mut self, protocol: u32, flags: u32, baud: u32) -> Result<u32, String> {
            self.cable_isotp = false;
            // Raw CAN with Wideband's own ISO-TP is the default. The cable's ISO15765 mode is
            // opt-in (`--cable-isotp`): on the Mini-VCI every empty read there stalls ~5 s.
            if protocol == 5
                && std::env::var_os("WIDEBAND_CABLE_ISOTP").is_some()
                && std::env::var_os("WIDEBAND_RAW_CAN").is_none()
            {
                match self.connect_cable_isotp(flags, baud) {
                    Ok(channel) => {
                        self.channel_id = channel;
                        self.active_protocol = protocol;
                        self.extended = self.connect_extended;
                        self.cable_isotp = true;
                        self.version_str = format!("{} · ISO 15765 (cable ISO-TP)", self.version_str);
                        return Ok(self.channel_id);
                    }
                    Err(e) => {
                        self.version_str = format!(
                            "{} · raw CAN (cable refused ISO 15765: {})",
                            self.version_str, e
                        );
                    }
                }
            }
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
            let wait = timeout_ms.min(MAX_READ_WAIT_MS);
            let resp = self.command(&format!("READ {}", wait), timeout_ms.max(50))?;
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

        fn ioctl(&mut self, _channel: u32, ioctl_id: u32, input: &[u8]) -> Result<Vec<u8>, String> {
            let line = if input.is_empty() {
                format!("IOCTL {:X}", ioctl_id)
            } else {
                format!("IOCTL {:X} {}", ioctl_id, bytes_to_hex(input))
            };
            let resp = self.command(&line, 15_000)?;
            Self::require_ok(&resp)?;
            let data_hex = resp.get("data").and_then(|v| v.as_str()).unwrap_or("");
            Ok(hex_to_bytes(data_hex))
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

/// Read-only bench check against a real cable and ECM (ignition ON). Skipped unless
/// `WIDEBAND_HW_DLL` names the J2534 DLL, e.g.
/// `WIDEBAND_HW_DLL=...\MVCI32.dll cargo test --release hardware_identify -- --ignored --nocapture`.
#[cfg(all(test, windows))]
mod hardware {
    use crate::protocols::{EcuIdentifier, EcuProfile};
    use std::time::Instant;

    #[test]
    #[ignore]
    fn hardware_identify_returns_vin_and_cal_id() {
        let dll = match std::env::var("WIDEBAND_HW_DLL") {
            Ok(path) => path,
            Err(_) => return,
        };
        let profile = EcuProfile::tacoma_can();
        let mut dev = super::create_host_device(&dll);
        dev.open().expect("open cable");
        let channel = crate::j2534::connect_for_profile(dev.as_mut(), &profile).expect("connect");
        let started = Instant::now();
        let result = EcuIdentifier::query(dev.as_mut(), channel, &profile, false);
        let elapsed = started.elapsed();
        let _ = dev.close();
        let id = result.expect("identify");
        println!("{} ({} ms)", id.details, elapsed.as_millis());
        assert_eq!(id.vin.len(), 17, "VIN: {:?}", id.vin);
        assert!(!id.cal_ids.is_empty(), "no Cal ID");
    }

    /// Read-only: Identify TCM on the ECM's connection, as the Transmission tab does it.
    #[test]
    #[ignore]
    fn hardware_identify_tcm() {
        let dll = match std::env::var("WIDEBAND_HW_DLL") {
            Ok(path) => path,
            Err(_) => return,
        };
        let profile = EcuProfile::tacoma_can();
        let tcm = crate::tcm::TcmProfile::tacoma_can_tcm();
        let mut dev = super::create_host_device(&dll);
        dev.open().expect("open cable");
        let channel = crate::j2534::connect_for_profile(dev.as_mut(), &profile).expect("connect");
        let mut client = crate::uds::UdsClient::for_profile(dev.as_mut(), channel, &tcm.to_diagnostic_profile());
        let tester = client.tester_present(dev.as_mut(), tcm.diagnostic);
        let started = Instant::now();
        let id = crate::tcm::TcmIdentifier::query(dev.as_mut(), channel, &tcm, false);
        let elapsed = started.elapsed();
        let _ = dev.close();
        println!("3E 00 on 0x{:X}: {:?}", tcm.request_id, tester);
        let id = id.expect("identify TCM");
        println!("{} ({} ms)", id.details, elapsed.as_millis());
        tester.expect("TCM TesterPresent");
        assert!(!id.cal_ids.is_empty(), "no TCM Cal ID");
    }

    /// Read-only: UDS TesterPresent on the diagnostic ids, then one OBD PID and one DID through
    /// the telemetry path.
    #[test]
    #[ignore]
    fn hardware_diagnostic_and_obd_ids_answer() {
        let dll = match std::env::var("WIDEBAND_HW_DLL") {
            Ok(path) => path,
            Err(_) => return,
        };
        let profile = EcuProfile::tacoma_can();
        let mut dev = super::create_host_device(&dll);
        dev.open().expect("open cable");
        let channel = crate::j2534::connect_for_profile(dev.as_mut(), &profile).expect("connect");
        let mut client = crate::uds::UdsClient::for_profile(dev.as_mut(), channel, &profile);
        let tester = client.tester_present(dev.as_mut(), profile.diagnostic);
        let rpm = crate::protocols::IsoTpLink::exchange_for_profile(dev.as_mut(), channel, &profile, &[0x01, 0x0C], 500);
        let did = crate::protocols::IsoTpLink::exchange_for_profile(dev.as_mut(), channel, &profile, &[0x22, 0x01, 0x03], 500);
        let _ = dev.close();
        println!("3E 00 on 0x{:X}: {:?}", profile.request_id, tester);
        println!("01 0C via 0x7DF: {:02X?}", rpm);
        println!("22 01 03 on 0x{:X}: {:02X?}", profile.request_id, did);
        tester.expect("TesterPresent on the diagnostic ids");
        assert_eq!(rpm.expect("Mode 01 PID 0C")[..2], [0x41, 0x0C]);
        assert_eq!(did.expect("DID 0103")[..3], [0x62, 0x01, 0x03]);
    }

    /// Read-only: every channel in telemetry-channels.json through the live-data path, then a
    /// burst of 10 ms RPM polls (leaves late replies in the buffer) followed by Identify.
    #[test]
    #[ignore]
    fn hardware_live_channels_then_identify_after_stale_replies() {
        let dll = match std::env::var("WIDEBAND_HW_DLL") {
            Ok(path) => path,
            Err(_) => return,
        };
        let channels = crate::telemetry::TelemetryChannelFile::load(std::path::Path::new("telemetry-channels.json"))
            .expect("telemetry-channels.json")
            .channels;
        let profile = EcuProfile::tacoma_can();
        let mut dev = super::create_host_device(&dll);
        dev.open().expect("open cable");
        let channel = crate::j2534::connect_for_profile(dev.as_mut(), &profile).expect("connect");
        let mut answered = 0;
        for ch in &channels {
            let req = ch.build_request_bytes();
            let started = Instant::now();
            let resp = crate::protocols::IsoTpLink::exchange_for_profile(dev.as_mut(), channel, &profile, &req, 100);
            let value = resp.as_ref().ok().and_then(|r| crate::telemetry::decode_channel_value(&ch.decode, r));
            if value.is_some() {
                answered += 1;
            }
            println!(
                "{:<18} {:02X?} -> {:02X?} value={:?} ({} ms)",
                ch.id,
                req,
                resp,
                value,
                started.elapsed().as_millis()
            );
        }
        for _ in 0..20 {
            let _ = crate::protocols::IsoTpLink::exchange_for_profile(dev.as_mut(), channel, &profile, &[0x01, 0x0C], 10);
        }
        let id = EcuIdentifier::query(dev.as_mut(), channel, &profile, false);
        let _ = dev.close();
        println!("{} of {} channels decoded", answered, channels.len());
        let id = id.expect("identify after stale replies");
        println!("{}", id.details);
        assert_eq!(id.vin.len(), 17, "VIN after stale replies: {:?}", id.details);
        assert!(answered > 0, "no live channel decoded");
    }
}

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

    pub fn missing_host_user_message() -> String {
        String::new()
    }
}
