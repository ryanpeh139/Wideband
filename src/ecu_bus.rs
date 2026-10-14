//! KWP2000 (K-Line) and J1850 VPW framing over J2534 raw `PassThruMsg` payloads.
//! Mirrors C# `EcuBus.cs` (`KwpFrame`, `VpwFrame`, `KLineLink`, `VpwLink`).

use crate::isotp::IsoTpLink;
use crate::j2534::{PassThruDevice, PassThruMsg, ISO14230, J1850VPW};
use crate::protocols::{EcuProfile, VehicleBus};
use std::thread;
use std::time::{Duration, Instant};

pub mod kwp {
    pub fn checksum(frame: &[u8], length: usize) -> u8 {
        let sum = frame.iter().take(length).fold(0u32, |acc, &b| acc + b as u32);
        (sum & 0xFF) as u8
    }

    pub fn pack(target: u8, source: u8, payload: &[u8]) -> Result<Vec<u8>, String> {
        if payload.is_empty() || payload.len() > 255 {
            return Err("KWP payload must be 1..255 bytes".to_string());
        }
        let extra = payload.len() > 63;
        let header = if extra { 4 } else { 3 };
        let mut frame = vec![0u8; header + payload.len() + 1];
        frame[0] = if extra { 0x80 } else { 0x80 | (payload.len() as u8) };
        frame[1] = target;
        frame[2] = source;
        let at = if extra {
            frame[3] = payload.len() as u8;
            4
        } else {
            3
        };
        frame[at..at + payload.len()].copy_from_slice(payload);
        let cs_index = frame.len() - 1;
        let cs = checksum(&frame, cs_index);
        frame[cs_index] = cs;
        Ok(frame)
    }

    fn parse_frame(frame: &[u8], expected_addr: u8) -> Option<Vec<u8>> {
        if frame.len() < 4 {
            return None;
        }
        let mode = frame[0] & 0xC0;
        if mode != 0x80 && mode != 0xC0 {
            return None;
        }
        let bits = frame[0] & 0x3F;
        let (header, length) = if bits == 0 {
            if frame.len() < 5 {
                return None;
            }
            (4, frame[3] as usize)
        } else {
            (3, bits as usize)
        };
        if length < 1 || frame[1] != expected_addr {
            return None;
        }
        let expect_with_cs = header + length + 1;
        let expect_no_cs = header + length;
        if frame.len() == expect_with_cs {
            if checksum(frame, frame.len() - 1) != frame[frame.len() - 1] {
                return None;
            }
        } else if frame.len() != expect_no_cs {
            return None;
        }
        Some(frame[header..header + length].to_vec())
    }

    /// Unpack a frame addressed to the tester (ECU → scan tool).
    pub fn try_unpack(frame: &[u8], tester: u8) -> Option<Vec<u8>> {
        parse_frame(frame, tester)
    }

    /// Unpack a frame addressed to the ECU (scan tool → ECU).
    pub fn try_unpack_to_ecu(frame: &[u8], ecu: u8) -> Option<Vec<u8>> {
        parse_frame(frame, ecu)
    }
}

pub mod vpw {
    pub fn pack(header: u8, target: u8, source: u8, payload: &[u8]) -> Result<Vec<u8>, String> {
        if payload.is_empty() || payload.len() > 8 {
            return Err("VPW payload must be 1..8 bytes".to_string());
        }
        let mut frame = vec![header, target, source];
        frame.extend_from_slice(payload);
        Ok(frame)
    }

    fn parse_frame(frame: &[u8], expected_target: u8) -> Option<Vec<u8>> {
        if frame.len() < 4 || frame.len() > 11 {
            return None;
        }
        if frame[1] != expected_target {
            return None;
        }
        let length = frame.len() - 3;
        Some(frame[3..3 + length].to_vec())
    }

    pub fn try_unpack(frame: &[u8], tester: u8) -> Option<Vec<u8>> {
        parse_frame(frame, tester)
    }

    pub fn try_unpack_to_ecu(frame: &[u8], ecu: u8) -> Option<Vec<u8>> {
        parse_frame(frame, ecu)
    }
}

enum Transport {
    IsoTp(IsoTpLink),
    KLine(KLineLink),
    Vpw(VpwLink),
}

/// Unified diagnostic transport for UDS/KWP request–response over CAN ISO-TP, K-Line, or VPW.
pub struct DiagnosticTransport {
    inner: Transport,
}

impl DiagnosticTransport {
    pub fn for_profile(channel: u32, profile: &EcuProfile) -> Self {
        let inner = match profile.bus {
            VehicleBus::CanIsoTp => Transport::IsoTp(IsoTpLink::new(
                channel,
                crate::isotp::IsoTpConfig {
                    tx_id: profile.request_id,
                    rx_id: profile.response_id,
                    pad_byte: 0xAA,
                    timeout_ms: 2000,
                    block_size: 0,
                    stmin_ms: 0,
                    max_flow_waits: 5,
                },
            )),
            VehicleBus::KLine => Transport::KLine(KLineLink::new(
                channel,
                profile.ecu,
                profile.tester,
                2000,
            )),
            VehicleBus::Vpw => Transport::Vpw(VpwLink::new(
                channel,
                (profile.request_id & 0xFF) as u8,
                profile.ecu,
                profile.tester,
                2000,
            )),
        };
        Self { inner }
    }

    pub fn set_timeout_ms(&mut self, timeout_ms: u32) {
        match &mut self.inner {
            Transport::IsoTp(l) => l.set_timeout_ms(timeout_ms),
            Transport::KLine(l) => l.set_timeout_ms(timeout_ms),
            Transport::Vpw(l) => l.set_timeout_ms(timeout_ms),
        }
    }

    pub fn send(&self, device: &mut dyn PassThruDevice, payload: &[u8]) -> Result<(), String> {
        match &self.inner {
            Transport::IsoTp(l) => l.send(device, payload),
            Transport::KLine(l) => l.send(device, payload),
            Transport::Vpw(l) => l.send(device, payload),
        }
    }

    pub fn receive(&self, device: &mut dyn PassThruDevice) -> Result<Vec<u8>, String> {
        match &self.inner {
            Transport::IsoTp(l) => l.receive(device),
            Transport::KLine(l) => l.receive(device),
            Transport::Vpw(l) => l.receive(device),
        }
    }
}

pub struct KLineLink {
    channel: u32,
    ecu: u8,
    tester: u8,
    timeout_ms: u32,
}

impl KLineLink {
    pub fn new(channel: u32, ecu: u8, tester: u8, timeout_ms: u32) -> Self {
        Self {
            channel,
            ecu,
            tester,
            timeout_ms,
        }
    }

    pub fn set_timeout_ms(&mut self, timeout_ms: u32) {
        if timeout_ms > 0 {
            self.timeout_ms = timeout_ms;
        }
    }

    pub fn send(&self, device: &mut dyn PassThruDevice, payload: &[u8]) -> Result<(), String> {
        let frame = kwp::pack(self.ecu, self.tester, payload)?;
        device.write_msgs(
            self.channel,
            &[PassThruMsg::new(ISO14230, &frame)],
            self.timeout_ms,
        )?;
        Ok(())
    }

    pub fn receive(&self, device: &mut dyn PassThruDevice) -> Result<Vec<u8>, String> {
        let deadline = Instant::now() + Duration::from_millis(self.timeout_ms as u64);
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now()).as_millis() as u32;
            let batch = device.read_msgs(self.channel, remaining.max(1))?;
            for msg in batch {
                let raw = msg.payload();
                if let Some(payload) = kwp::try_unpack(raw, self.tester) {
                    return Ok(payload);
                }
            }
            thread::sleep(Duration::from_millis(1));
        }
        Err("K-Line read timed out".to_string())
    }
}

pub struct VpwLink {
    channel: u32,
    header: u8,
    ecu: u8,
    tester: u8,
    timeout_ms: u32,
}

impl VpwLink {
    pub fn new(channel: u32, header: u8, ecu: u8, tester: u8, timeout_ms: u32) -> Self {
        Self {
            channel,
            header,
            ecu,
            tester,
            timeout_ms,
        }
    }

    pub fn set_timeout_ms(&mut self, timeout_ms: u32) {
        if timeout_ms > 0 {
            self.timeout_ms = timeout_ms;
        }
    }

    pub fn send(&self, device: &mut dyn PassThruDevice, payload: &[u8]) -> Result<(), String> {
        let frame = vpw::pack(self.header, self.ecu, self.tester, payload)?;
        device.write_msgs(
            self.channel,
            &[PassThruMsg::new(J1850VPW, &frame)],
            self.timeout_ms,
        )?;
        Ok(())
    }

    pub fn receive(&self, device: &mut dyn PassThruDevice) -> Result<Vec<u8>, String> {
        let deadline = Instant::now() + Duration::from_millis(self.timeout_ms as u64);
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now()).as_millis() as u32;
            let batch = device.read_msgs(self.channel, remaining.max(1))?;
            for msg in batch {
                let raw = msg.payload();
                if let Some(payload) = vpw::try_unpack(raw, self.tester) {
                    return Ok(payload);
                }
            }
            thread::sleep(Duration::from_millis(1));
        }
        Err("VPW read timed out".to_string())
    }
}

/// OBD/diagnostic payload exchange (any bus).
pub fn exchange(
    device: &mut dyn PassThruDevice,
    channel: u32,
    profile: &EcuProfile,
    request: &[u8],
) -> Result<Vec<u8>, String> {
    let mut link = DiagnosticTransport::for_profile(channel, profile);
    link.set_timeout_ms(2500);
    link.send(device, request)?;
    link.receive(device)
}

#[cfg(test)]
pub(crate) mod mock {
    use super::*;
    use crate::j2534::PassThruDevice;
    use std::collections::VecDeque;

    pub struct ScriptRawBus {
        pub protocol_id: u32,
        pub ecu: u8,
        pub tester: u8,
        pub vpw_header: u8,
        pub connected: bool,
        pub rx_script: VecDeque<Vec<u8>>,
        pub tx_captured: Vec<Vec<u8>>,
    }

    impl ScriptRawBus {
        pub fn kline(ecu: u8, tester: u8) -> Self {
            Self {
                protocol_id: ISO14230,
                ecu,
                tester,
                vpw_header: 0x6C,
                connected: true,
                rx_script: VecDeque::new(),
                tx_captured: Vec::new(),
            }
        }

        pub fn enqueue_rx_payload(&mut self, payload: &[u8]) {
            let frame = if self.protocol_id == ISO14230 {
                kwp::pack(self.tester, self.ecu, payload).expect("pack")
            } else {
                vpw::pack(self.vpw_header, self.tester, self.ecu, payload).expect("pack")
            };
            self.rx_script.push_back(frame);
        }
    }

    impl PassThruDevice for ScriptRawBus {
        fn open(&mut self) -> Result<(), String> {
            self.connected = true;
            Ok(())
        }
        fn close(&mut self) -> Result<(), String> {
            self.connected = false;
            Ok(())
        }
        fn connect(&mut self, protocol: u32, _flags: u32, _baud: u32) -> Result<u32, String> {
            self.protocol_id = protocol;
            Ok(1)
        }
        fn disconnect(&mut self, _channel: u32) -> Result<(), String> {
            Ok(())
        }
        fn read_msgs(&mut self, _channel: u32, _timeout_ms: u32) -> Result<Vec<PassThruMsg>, String> {
            if let Some(frame) = self.rx_script.pop_front() {
                Ok(vec![PassThruMsg::new(self.protocol_id, &frame)])
            } else {
                Ok(Vec::new())
            }
        }
        fn write_msgs(&mut self, _channel: u32, msgs: &[PassThruMsg], _timeout_ms: u32) -> Result<u32, String> {
            for msg in msgs {
                self.tx_captured.push(msg.payload().to_vec());
            }
            Ok(msgs.len() as u32)
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
        fn is_connected(&self) -> bool {
            self.connected
        }
        fn firmware_version(&self) -> String {
            "script-raw-bus".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mock::ScriptRawBus;

    #[test]
    fn kwp_roundtrip() {
        let payload = vec![0x01, 0x03];
        let tx = kwp::pack(0x10, 0xF1, &payload).unwrap();
        assert_eq!(kwp::try_unpack_to_ecu(&tx, 0x10).unwrap(), payload);
        let rx = kwp::pack(0xF1, 0x10, &payload).unwrap();
        assert_eq!(kwp::try_unpack(&rx, 0xF1).unwrap(), payload);
    }

    #[test]
    fn vpw_roundtrip() {
        let payload = vec![0x01, 0x01];
        let tx = vpw::pack(0x6C, 0x10, 0xF1, &payload).unwrap();
        assert_eq!(vpw::try_unpack_to_ecu(&tx, 0x10).unwrap(), payload);
        let rx = vpw::pack(0x6C, 0xF1, 0x10, &payload).unwrap();
        assert_eq!(vpw::try_unpack(&rx, 0xF1).unwrap(), payload);
    }

    #[test]
    fn kline_link_scripted_exchange() {
        let mut dev = ScriptRawBus::kline(0x10, 0xF1);
        dev.enqueue_rx_payload(&[0x43, 0x01, 0x01, 0x71]);
        let prof = EcuProfile::tacoma_kline();
        let resp = exchange(&mut dev, 1, &prof, &[0x03]).expect("mode 03");
        assert_eq!(resp[0], 0x43);
        assert!(!dev.tx_captured.is_empty());
    }
}
