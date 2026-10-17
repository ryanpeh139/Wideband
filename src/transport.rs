//! Pluggable link transports for Motorsport Studio (USB J2534, Bluetooth serial, Ethernet).
//!
//! USB J2534 continues to use `j2534::create_device` → `J2534Host.exe` IPC when available.
//! Wireless backends implement [`WirelessPassthruAdapter`] (see `docs/wireless-transport-adapters.md`).

use crate::j2534::{
    create_device, transport_hint_for_device, J2534DeviceInfo, J2534RegistryScanner, PassThruDevice,
};
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

/// Physical / logical link used to reach the vehicle bus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportKind {
    /// Bench-only synthetic ECU (not an OBD cable).
    VirtualBench,
    /// SAE J2534 Pass-Thru over USB (32-bit DLL via host bridge on 64-bit builds).
    UsbJ2534,
    /// RFCOMM / BLE-UART serial bridge (ESP32, ELM327-style gateways).
    BluetoothSerial,
    /// TCP JSON-line gateway exposing the same command set as `J2534Host.exe`.
    EthernetGateway,
}

/// Discovery / selection state shown in the device combo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransportAvailability {
    /// Ready to connect (DLL on disk, gateway probed, or virtual bench).
    Ready,
    /// Configured endpoint not reachable (power, pairing, Wi-Fi).
    Offline,
    /// J2534 registry entry with missing `FunctionLibrary`.
    MissingDll,
    /// Placeholder row — set `WIDEBAND_BT_SERIAL` / `WIDEBAND_ETH_GATEWAYS` (see docs).
    NotConfigured,
    /// Last open attempt failed; message is shown in the header chip.
    Error(String),
}

/// Connection parameters for [`TransportDevice::open_pass_thru`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransportEndpoint {
    VirtualBench,
    J2534Usb { library_path: String },
    BluetoothSerial { port_name: String },
    EthernetGateway { host: String, port: u16 },
}

/// Unified device row for UI discovery (USB + wireless + virtual).
#[derive(Clone, Debug)]
pub struct TransportDevice {
    pub id: String,
    pub display_name: String,
    pub vendor: String,
    pub kind: TransportKind,
    pub availability: TransportAvailability,
    pub endpoint: TransportEndpoint,
}

impl TransportDevice {
    pub fn list_label(&self) -> String {
        let kind_tag = match self.kind {
            TransportKind::VirtualBench => "Virtual",
            TransportKind::UsbJ2534 => "USB J2534",
            TransportKind::BluetoothSerial => "Bluetooth",
            TransportKind::EthernetGateway => "Ethernet",
        };
        let suffix = match &self.availability {
            TransportAvailability::Ready => "",
            TransportAvailability::Offline => " — offline",
            TransportAvailability::MissingDll => " — DLL not found",
            TransportAvailability::NotConfigured => " — not configured",
            TransportAvailability::Error(msg) => {
                if msg.len() > 48 {
                    return format!("[{}] {} — error", kind_tag, self.display_name);
                }
                return format!("[{}] {} — {}", kind_tag, self.display_name, msg);
            }
        };
        format!("[{}] {}{}", kind_tag, self.display_name, suffix)
    }

    pub fn is_virtual_bench(&self) -> bool {
        self.kind == TransportKind::VirtualBench
    }

    /// Index to preselect: the first ready USB J2534 cable, else the first row (Virtual ECU).
    /// The Virtual ECU stays an explicit choice whenever a real cable is installed.
    pub fn preferred_index(devices: &[TransportDevice]) -> usize {
        devices
            .iter()
            .position(|d| d.kind == TransportKind::UsbJ2534 && d.availability == TransportAvailability::Ready)
            .unwrap_or(0)
    }

    pub fn is_connectable(&self) -> bool {
        if self.kind == TransportKind::VirtualBench {
            return self.availability == TransportAvailability::Ready;
        }
        matches!(
            self.availability,
            TransportAvailability::Ready | TransportAvailability::Error(_)
        )
    }

    pub fn transport_hint(&self) -> &'static str {
        match self.endpoint {
            TransportEndpoint::VirtualBench => "virtual loopback (bench only)",
            TransportEndpoint::J2534Usb { .. } => {
                let info = J2534DeviceInfo {
                    name: self.display_name.clone(),
                    vendor: self.vendor.clone(),
                    library_path: j2534_path_from_endpoint(&self.endpoint),
                    is_virtual: false,
                    exists: self.availability == TransportAvailability::Ready,
                };
                transport_hint_for_device(&info)
            }
            TransportEndpoint::BluetoothSerial { .. } => "Bluetooth serial adapter (RFCOMM / BLE-UART)",
            TransportEndpoint::EthernetGateway { .. } => {
                "Ethernet JSON-line gateway (J2534Host-compatible)"
            }
        }
    }

    pub fn open_pass_thru(self) -> Result<Box<dyn PassThruDevice>, String> {
        match self.endpoint {
            TransportEndpoint::VirtualBench => Ok(create_device(&virtual_j2534_info())),
            TransportEndpoint::J2534Usb { library_path } => {
                if !std::path::Path::new(&library_path).exists() {
                    return Err(format!("PassThru DLL not found: {}", library_path));
                }
                Ok(create_device(&J2534DeviceInfo {
                    name: self.display_name.clone(),
                    vendor: self.vendor.clone(),
                    library_path,
                    is_virtual: false,
                    exists: true,
                }))
            }
            TransportEndpoint::BluetoothSerial { port_name } => {
                StubWirelessAdapter::bluetooth_serial().open_bluetooth_serial(&port_name)
            }
            TransportEndpoint::EthernetGateway { host, port } => {
                StubWirelessAdapter::ethernet_gateway().open_ethernet_gateway(&host, port)
            }
        }
    }
}

fn j2534_path_from_endpoint(endpoint: &TransportEndpoint) -> String {
    match endpoint {
        TransportEndpoint::J2534Usb { library_path } => library_path.clone(),
        _ => String::new(),
    }
}

fn virtual_j2534_info() -> J2534DeviceInfo {
    J2534DeviceInfo {
        name: "Wideband Virtual ECU Simulator (Loopback)".to_string(),
        vendor: "Wideband Bench Suite".to_string(),
        library_path: "virtual".to_string(),
        is_virtual: true,
        exists: true,
    }
}

pub struct TransportRegistry;

impl TransportRegistry {
    pub fn discover() -> Vec<TransportDevice> {
        let mut out = Vec::new();
        out.push(virtual_device());
        for j in J2534RegistryScanner::scan() {
            if j.is_virtual {
                continue;
            }
            out.push(from_j2534(&j));
        }
        for dev in discover_bluetooth_serial() {
            out.push(dev);
        }
        for dev in discover_ethernet_gateways() {
            out.push(dev);
        }
        if !out.iter().any(|d| d.kind == TransportKind::BluetoothSerial) {
            out.push(bluetooth_placeholder());
        }
        if !out.iter().any(|d| d.kind == TransportKind::EthernetGateway) {
            out.push(ethernet_placeholder());
        }
        out
    }
}

fn virtual_device() -> TransportDevice {
    TransportDevice {
        id: "virtual-bench".to_string(),
        display_name: "Offline loopback ECU (practice ladder)".to_string(),
        vendor: "Wideband Bench Suite".to_string(),
        kind: TransportKind::VirtualBench,
        availability: TransportAvailability::Ready,
        endpoint: TransportEndpoint::VirtualBench,
    }
}

fn from_j2534(info: &J2534DeviceInfo) -> TransportDevice {
    let availability = if info.exists {
        TransportAvailability::Ready
    } else {
        TransportAvailability::MissingDll
    };
    TransportDevice {
        id: format!("j2534:{}", info.library_path),
        display_name: info.name.clone(),
        vendor: info.vendor.clone(),
        kind: TransportKind::UsbJ2534,
        availability,
        endpoint: TransportEndpoint::J2534Usb {
            library_path: info.library_path.clone(),
        },
    }
}

fn bluetooth_placeholder() -> TransportDevice {
    TransportDevice {
        id: "bt:unconfigured".to_string(),
        display_name: "Bluetooth OBD serial (configure env)".to_string(),
        vendor: "Wireless adapter".to_string(),
        kind: TransportKind::BluetoothSerial,
        availability: TransportAvailability::NotConfigured,
        endpoint: TransportEndpoint::BluetoothSerial {
            port_name: String::new(),
        },
    }
}

fn ethernet_placeholder() -> TransportDevice {
    TransportDevice {
        id: "eth:unconfigured".to_string(),
        display_name: "Ethernet/Wi-Fi gateway (configure env)".to_string(),
        vendor: "Wireless adapter".to_string(),
        kind: TransportKind::EthernetGateway,
        availability: TransportAvailability::NotConfigured,
        endpoint: TransportEndpoint::EthernetGateway {
            host: String::new(),
            port: 0,
        },
    }
}

fn discover_bluetooth_serial() -> Vec<TransportDevice> {
    let raw = std::env::var("WIDEBAND_BT_SERIAL").unwrap_or_default();
    parse_csv_ports(&raw)
        .into_iter()
        .map(|port| {
            TransportDevice {
                id: format!("bt:{}", port),
                display_name: format!("Serial bridge on {}", port),
                vendor: "Bluetooth / RFCOMM".to_string(),
                kind: TransportKind::BluetoothSerial,
                availability: TransportAvailability::Ready,
                endpoint: TransportEndpoint::BluetoothSerial {
                    port_name: port,
                },
            }
        })
        .collect()
}

fn discover_ethernet_gateways() -> Vec<TransportDevice> {
    let raw = std::env::var("WIDEBAND_ETH_GATEWAYS").unwrap_or_default();
    let mut out = Vec::new();
    for token in raw.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
        if let Some((host, port)) = parse_host_port(token) {
            let availability = if probe_gateway(&host, port) {
                TransportAvailability::Ready
            } else {
                TransportAvailability::Offline
            };
            out.push(TransportDevice {
                id: format!("eth:{}:{}", host, port),
                display_name: format!("Gateway {}:{}", host, port),
                vendor: "Ethernet / Wi-Fi".to_string(),
                kind: TransportKind::EthernetGateway,
                availability,
                endpoint: TransportEndpoint::EthernetGateway { host, port },
            });
        }
    }
    out
}

fn parse_csv_ports(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

fn parse_host_port(token: &str) -> Option<(String, u16)> {
    if let Some((host, port_str)) = token.rsplit_once(':') {
        if host.is_empty() {
            return None;
        }
        let port = port_str.parse().ok()?;
        return Some((host.to_string(), port));
    }
    None
}

fn probe_gateway(host: &str, port: u16) -> bool {
    let addr: SocketAddr = match format!("{}:{}", host, port).parse() {
        Ok(a) => a,
        Err(_) => return false,
    };
    TcpStream::connect_timeout(&addr, Duration::from_millis(400)).is_ok()
}

/// Contract for vendor wireless adapters. Default implementations are stubs until an SDK is wired.
pub trait WirelessPassthruAdapter: Send + Sync {
    fn open_bluetooth_serial(&self, port_name: &str) -> Result<Box<dyn PassThruDevice>, String>;
    fn open_ethernet_gateway(&self, host: &str, port: u16) -> Result<Box<dyn PassThruDevice>, String>;
}

pub struct StubWirelessAdapter;

impl StubWirelessAdapter {
    pub fn bluetooth_serial() -> &'static StubWirelessAdapter {
        static ADAPTER: StubWirelessAdapter = StubWirelessAdapter;
        &ADAPTER
    }

    pub fn ethernet_gateway() -> &'static StubWirelessAdapter {
        static ADAPTER: StubWirelessAdapter = StubWirelessAdapter;
        &ADAPTER
    }
}

impl WirelessPassthruAdapter for StubWirelessAdapter {
    fn open_bluetooth_serial(&self, port_name: &str) -> Result<Box<dyn PassThruDevice>, String> {
        if port_name.is_empty() {
            return Err(
                "Bluetooth serial not configured. Set WIDEBAND_BT_SERIAL=COM7 (Windows) or /dev/rfcomm0 (Linux)."
                    .to_string(),
            );
        }
        Err(format!(
            "Bluetooth serial adapter stub: no RFCOMM backend linked for '{}'. \
Implement WirelessPassthruAdapter or use USB J2534. See docs/wireless-transport-adapters.md.",
            port_name
        ))
    }

    fn open_ethernet_gateway(&self, host: &str, port: u16) -> Result<Box<dyn PassThruDevice>, String> {
        if host.is_empty() || port == 0 {
            return Err(
                "Ethernet gateway not configured. Set WIDEBAND_ETH_GATEWAYS=192.168.4.1:35000."
                    .to_string(),
            );
        }
        RemoteLinePassthru::connect(host, port)
    }
}

/// TCP client speaking newline-delimited JSON commands (same shape as `J2534Host.exe` stdout).
struct RemoteLinePassthru {
    stream: TcpStream,
    reader: BufReader<TcpStream>,
    connected: bool,
    channel_id: u32,
    version_str: String,
    connect_extended: bool,
    connect_filter_id: u32,
    connect_filter_mask: u32,
}

impl RemoteLinePassthru {
    fn connect(host: &str, port: u16) -> Result<Box<dyn PassThruDevice>, String> {
        let addr: SocketAddr = format!("{}:{}", host, port)
            .parse()
            .map_err(|e| format!("Invalid gateway address: {}", e))?;
        let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(3))
            .map_err(|e| format!("Ethernet gateway unreachable at {}:{} ({})", host, port, e))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(15)))
            .map_err(|e| e.to_string())?;
        stream
            .set_write_timeout(Some(Duration::from_secs(15)))
            .map_err(|e| e.to_string())?;
        let reader_stream = stream.try_clone().map_err(|e| e.to_string())?;
        Ok(Box::new(Self {
            stream,
            reader: BufReader::new(reader_stream),
            connected: false,
            channel_id: 0,
            version_str: format!("Ethernet gateway {}:{}", host, port),
            connect_extended: false,
            connect_filter_id: 0x7E8,
            connect_filter_mask: 0x7FF,
        }))
    }

    fn command(&mut self, line: &str, timeout_ms: u32) -> Result<serde_json::Value, String> {
        writeln!(self.stream, "{}", line).map_err(|e| format!("gateway write failed: {}", e))?;
        self.stream.flush().map_err(|e| e.to_string())?;
        let deadline =
            std::time::Instant::now() + Duration::from_millis(timeout_ms.max(50) as u64);
        while std::time::Instant::now() < deadline {
            let mut line_buf = String::new();
            match self.reader.read_line(&mut line_buf) {
                Ok(0) => break,
                Ok(_) => {
                    if let Some(start) = line_buf.find('{') {
                        if let Some(end) = line_buf.rfind('}') {
                            let json = &line_buf[start..=end];
                            return serde_json::from_str(json).map_err(|e| e.to_string());
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {
                    continue;
                }
                Err(e) => return Err(format!("gateway read failed: {}", e)),
            }
        }
        Err("Ethernet gateway response timed out".to_string())
    }

    fn require_ok(v: &serde_json::Value) -> Result<(), String> {
        if v.get("ok").and_then(|b| b.as_bool()) == Some(true) {
            return Ok(());
        }
        let err = v
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("gateway error");
        Err(err.to_string())
    }
}

impl PassThruDevice for RemoteLinePassthru {
    fn set_connect_hints(&mut self, extended: bool, filter_id: u32, filter_mask: u32) {
        self.connect_extended = extended;
        self.connect_filter_id = filter_id;
        self.connect_filter_mask = filter_mask;
    }

    fn open(&mut self) -> Result<(), String> {
        let dll = std::env::var("WIDEBAND_GATEWAY_DLL").unwrap_or_else(|_| "gateway".to_string());
        let quoted = if dll.contains(' ') {
            format!("\"{}\"", dll)
        } else {
            dll
        };
        let resp = self.command(&format!("OPEN {}", quoted), 15_000)?;
        Self::require_ok(&resp)?;
        self.connected = true;
        if let Ok(ver) = self.command("VERSION", 5_000) {
            if ver.get("ok").and_then(|b| b.as_bool()) == Some(true) {
                let fw = ver.get("firmware").and_then(|v| v.as_str()).unwrap_or("?");
                self.version_str = format!("Gateway FW:{}", fw);
            }
        }
        Ok(())
    }

    fn close(&mut self) -> Result<(), String> {
        if self.connected {
            let _ = self.command("CLOSE", 3_000);
            let _ = self.command("QUIT", 1_000);
        }
        self.connected = false;
        self.channel_id = 0;
        Ok(())
    }

    fn connect(&mut self, protocol: u32, flags: u32, baud: u32) -> Result<u32, String> {
        let ext_flag = if self.connect_extended { 1 } else { 0 };
        let line = format!("CONNECT {} {} {} {:X}", baud, ext_flag, protocol, flags);
        let resp = self.command(&line, 15_000)?;
        Self::require_ok(&resp)?;
        self.channel_id = resp.get("channel_id").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        if protocol == crate::j2534::ISO15765 {
            let filter_line = format!(
                "FILTER {:X} {:X}",
                self.connect_filter_id,
                self.connect_filter_mask
            );
            let f = self.command(&filter_line, 15_000)?;
            Self::require_ok(&f)?;
        }
        Ok(self.channel_id)
    }

    fn disconnect(&mut self, _channel: u32) -> Result<(), String> {
        Ok(())
    }

    fn read_msgs(&mut self, _channel: u32, timeout_ms: u32) -> Result<Vec<crate::j2534::PassThruMsg>, String> {
        use crate::j2534::PassThruMsg;
        let resp = self.command("READ", timeout_ms.max(50))?;
        Self::require_ok(&resp)?;
        if resp.get("empty").and_then(|v| v.as_bool()) == Some(true) {
            return Ok(Vec::new());
        }
        let id_hex = resp.get("id").and_then(|v| v.as_str()).unwrap_or("0");
        let id = u32::from_str_radix(id_hex, 16).unwrap_or(0);
        let data_hex = resp.get("data").and_then(|v| v.as_str()).unwrap_or("");
        let payload = hex_to_bytes(data_hex);
        let mut msg = PassThruMsg::new_can(id, &[]);
        msg.write_id(id);
        let len = payload.len().min(4124);
        msg.data_size = (4 + len) as u32;
        msg.data[4..4 + len].copy_from_slice(&payload[..len]);
        Ok(vec![msg])
    }

    fn write_msgs(
        &mut self,
        _channel: u32,
        msgs: &[crate::j2534::PassThruMsg],
        timeout_ms: u32,
    ) -> Result<u32, String> {
        let mut count = 0u32;
        for msg in msgs {
            let id = msg.read_id();
            let payload = msg.payload();
            let line = format!("WRITE {:X} {}", id, bytes_to_hex(payload));
            let resp = self.command(&line, timeout_ms.max(1000))?;
            Self::require_ok(&resp)?;
            count += 1;
        }
        Ok(count)
    }

    fn start_filter(
        &mut self,
        _channel: u32,
        _filter_type: u32,
        _mask: &crate::j2534::PassThruMsg,
        _pattern: &crate::j2534::PassThruMsg,
        _flow_control: Option<&crate::j2534::PassThruMsg>,
    ) -> Result<u32, String> {
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

    fn channel_id(&self) -> u32 {
        if self.channel_id != 0 {
            self.channel_id
        } else {
            1
        }
    }

    fn device_label(&self) -> String {
        "ethernet-gateway".to_string()
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_list_label_prefix() {
        let v = virtual_device();
        assert!(v.list_label().starts_with("[Virtual]"));
        assert!(v.is_virtual_bench());
    }

    #[test]
    fn j2534_missing_dll_label() {
        let dev = from_j2534(&J2534DeviceInfo {
            name: "Test".into(),
            vendor: "V".into(),
            library_path: r"C:\missing.dll".into(),
            is_virtual: false,
            exists: false,
        });
        assert!(dev.list_label().contains("DLL not found"));
        assert_eq!(dev.kind, TransportKind::UsbJ2534);
    }

    #[test]
    fn preferred_index_picks_ready_cable_over_virtual() {
        let missing = from_j2534(&J2534DeviceInfo {
            name: "Gone".into(),
            vendor: "V".into(),
            library_path: r"C:\missing.dll".into(),
            is_virtual: false,
            exists: false,
        });
        let mut ready = missing.clone();
        ready.display_name = "MVCI Autokent-X64 - 1.4.3".into();
        ready.availability = TransportAvailability::Ready;
        assert_eq!(TransportDevice::preferred_index(&[virtual_device(), missing.clone()]), 0);
        assert_eq!(TransportDevice::preferred_index(&[virtual_device(), missing, ready]), 2);
    }

    #[test]
    fn parse_host_port_valid() {
        assert_eq!(
            parse_host_port("192.168.4.1:35000"),
            Some(("192.168.4.1".into(), 35000))
        );
        assert!(parse_host_port("nocolon").is_none());
    }

    #[test]
    fn discover_includes_virtual_and_placeholders() {
        let list = TransportRegistry::discover();
        assert!(list.iter().any(|d| d.kind == TransportKind::VirtualBench));
        assert!(list.iter().any(|d| d.availability == TransportAvailability::NotConfigured));
    }

    #[test]
    fn bluetooth_stub_requires_port() {
        let err = StubWirelessAdapter
            .open_bluetooth_serial("")
            .err()
            .expect("expected error");
        assert!(err.contains("WIDEBAND_BT_SERIAL"));
    }
}
