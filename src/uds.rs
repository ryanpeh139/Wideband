use crate::ecu_bus::DiagnosticTransport;
use crate::j2534::PassThruDevice;
use crate::protocols::{DiagnosticProtocol, EcuProfile};

const NEGATIVE: u8 = 0x7F;
const RESPONSE_PENDING: u8 = 0x78;
const TESTER_PRESENT_POSITIVE: u8 = 0x7E;
/// ECU-reported P2 is server-side only; the client adds cable + J2534Host IPC latency on top.
const CLIENT_P2_MARGIN_MS: u32 = 500;
/// Floor for client P2 so a 50 ms server P2 never becomes a 50 ms client deadline.
const MIN_CLIENT_P2_MS: u32 = 1000;
/// ISO 14229 floor for P2* (enhanced timeout after NRC 0x78).
const MIN_CLIENT_P2_STAR_MS: u32 = 5000;

pub struct UdsClient {
    link: DiagnosticTransport,
    max_pending: u32,
    p2_ms: u32,
    p2_star_ms: u32,
}

impl UdsClient {
    pub fn for_profile(_device: &mut dyn PassThruDevice, channel: u32, profile: &EcuProfile) -> Self {
        Self {
            link: DiagnosticTransport::for_profile(channel, profile),
            max_pending: 10,
            p2_ms: 2000,
            p2_star_ms: 5000,
        }
    }

    pub fn set_timers(&mut self, p2_ms: u32, p2_star_ms: u32) {
        if p2_ms > 0 {
            self.p2_ms = p2_ms;
        }
        if p2_star_ms > 0 {
            self.p2_star_ms = p2_star_ms;
        }
    }

    /// Apply ECU-reported server timing (from `diagnostic_session_control`) with client margin.
    pub fn apply_server_timing(&mut self, p2_server_ms: u32, p2_star_server_ms: u32) {
        self.p2_ms = p2_server_ms
            .saturating_add(CLIENT_P2_MARGIN_MS)
            .max(MIN_CLIENT_P2_MS);
        self.p2_star_ms = p2_star_server_ms
            .saturating_add(CLIENT_P2_MARGIN_MS)
            .max(MIN_CLIENT_P2_STAR_MS);
    }

    pub fn p2_ms(&self) -> u32 {
        self.p2_ms
    }

    pub fn p2_star_ms(&self) -> u32 {
        self.p2_star_ms
    }

    /// UDS `3E 80` (suppressPosRspMsgIndicationBit): send-only keepalive, never waits on the ECU.
    pub fn tester_present_no_response(&mut self, device: &mut dyn PassThruDevice) -> Result<(), String> {
        self.link.send(device, &[0x3E, 0x80])
    }

    pub fn tester_present(&mut self, device: &mut dyn PassThruDevice, protocol: DiagnosticProtocol) -> Result<(), String> {
        let req = match protocol {
            DiagnosticProtocol::Kwp => vec![0x3E],
            DiagnosticProtocol::Uds => vec![0x3E, 0x00],
        };
        let resp = self.request(device, &req, 0x7E)?;
        if protocol == DiagnosticProtocol::Uds && (resp.len() < 2 || resp[1] != 0x00) {
            return Err("TesterPresent response invalid".to_string());
        }
        Ok(())
    }

    pub fn diagnostic_session_control(
        &mut self,
        device: &mut dyn PassThruDevice,
        session: u8,
    ) -> Result<(u32, u32), String> {
        let resp = self.request(device, &[0x10, session], 0x50)?;
        // ISO 14229-1: P2server_max in 1 ms units, P2*server_max in 10 ms units.
        let p2 = if resp.len() >= 4 {
            ((resp[2] as u32) << 8) | (resp[3] as u32)
        } else {
            self.p2_ms
        };
        let p2_star = if resp.len() >= 6 {
            (((resp[4] as u32) << 8) | (resp[5] as u32)) * 10
        } else {
            self.p2_star_ms
        };
        Ok((p2, p2_star))
    }

    pub fn request(&mut self, device: &mut dyn PassThruDevice, request: &[u8], positive_service: u8) -> Result<Vec<u8>, String> {
        if request.is_empty() {
            return Err("Empty UDS request".to_string());
        }
        self.link.send(device, request)?;
        let response = self.receive_with_pending(device, request[0])?;
        if response.len() >= 3 && response[0] == NEGATIVE {
            if response[1] != request[0] {
                return Err("ECU negative response for different service".to_string());
            }
            return Err(format!("ECU NRC 0x{:02X}", response[2]));
        }
        if response.is_empty() || response[0] != positive_service {
            return Err(format!("Unexpected response to 0x{:02X}: {:?}", request[0], response));
        }
        Ok(response)
    }

    fn receive_with_pending(&mut self, device: &mut dyn PassThruDevice, service: u8) -> Result<Vec<u8>, String> {
        let mut pending = 0u32;
        let mut wait_ms = self.p2_ms;
        while pending <= self.max_pending {
            self.link.set_timeout_ms(wait_ms);
            let response = self.link.receive(device)?;
            // Late reply to a suppressed idle keepalive (`3E 80`): ECUs may still send `7E 00`
            // or a `7F 3E xx`. Drop it instead of failing the real request.
            if service != 0x3E
                && (response.first() == Some(&TESTER_PRESENT_POSITIVE)
                    || (response.len() >= 3 && response[0] == NEGATIVE && response[1] == 0x3E))
            {
                pending += 1;
                continue;
            }
            if response.len() >= 3 && response[0] == NEGATIVE && response[2] == RESPONSE_PENDING {
                if response[1] != service {
                    return Err("Pending response for different service".to_string());
                }
                pending += 1;
                wait_ms = extended_wait(self.p2_star_ms);
                continue;
            }
            return Ok(response);
        }
        Err("ECU response still pending (0x78 limit)".to_string())
    }

    pub fn read_memory(
        &mut self,
        device: &mut dyn PassThruDevice,
        profile: &EcuProfile,
        address: u64,
        count: usize,
    ) -> Result<Vec<u8>, String> {
        let addr_bytes = profile.address_bytes;
        let size_bytes = profile.size_bytes;
        if count == 0 {
            return Err("Read count must be > 0".to_string());
        }
        let mut req = vec![0x23, format_field(size_bytes, addr_bytes)];
        req.extend_from_slice(&encode_be(address, addr_bytes));
        req.extend_from_slice(&encode_be(count as u64, size_bytes));
        let resp = self.request(device, &req, 0x63)?;
        if resp.len() != 1 + count {
            return Err(format!("Read length mismatch: expected {}, got {}", count, resp.len() - 1));
        }
        Ok(resp[1..].to_vec())
    }

    pub fn read_range(
        &mut self,
        device: &mut dyn PassThruDevice,
        profile: &EcuProfile,
        address: u64,
        size: usize,
        chunk_bytes: usize,
        mut progress: impl FnMut(usize, usize),
    ) -> Result<Vec<u8>, String> {
        if size == 0 {
            return Err("Read range size must be > 0".to_string());
        }
        let chunk = chunk_bytes.max(1).min(profile.max_chunk);
        let mut out = vec![0u8; size];
        let mut offset = 0usize;
        while offset < size {
            let this = (size - offset).min(chunk);
            let chunk_data = self.read_memory(device, profile, address + offset as u64, this)?;
            out[offset..offset + this].copy_from_slice(&chunk_data);
            offset += this;
            progress(offset, size);
        }
        Ok(out)
    }
}

fn format_field(size_bytes: usize, address_bytes: usize) -> u8 {
    (((size_bytes & 0x0F) << 4) | (address_bytes & 0x0F)) as u8
}

fn encode_be(value: u64, bytes: usize) -> Vec<u8> {
    let mut out = vec![0u8; bytes];
    for i in 0..bytes {
        let shift = (bytes - 1 - i) * 8;
        out[i] = ((value >> shift) & 0xFF) as u8;
    }
    out
}

fn extended_wait(p2_star_ms: u32) -> u32 {
    if p2_star_ms < 5000 {
        5000
    } else {
        p2_star_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isotp::mock::ScriptCan;
    use crate::protocols::EcuProfile;

    #[test]
    fn uds_handles_response_pending_nrc() {
        let mut dev = ScriptCan::new(0x7E0, 0x7E8);
        dev.enqueue_rx(&[0x03, 0x7F, 0x10, 0x78, 0xAA, 0xAA, 0xAA, 0xAA]);
        dev.enqueue_rx(&[0x06, 0x50, 0x03, 0x00, 0x32, 0x01, 0xF4, 0xAA]);

        let prof = EcuProfile::tacoma_can();
        let mut client = UdsClient::for_profile(&mut dev, 1, &prof);
        client.set_timers(200, 200);
        let (p2, p2_star) = client
            .diagnostic_session_control(&mut dev, 0x03)
            .expect("pending then positive");
        assert_eq!(p2, 50);
        assert_eq!(p2_star, 5000, "P2* is reported in 10 ms units");
    }

    #[test]
    fn server_timing_gets_client_margin() {
        let mut dev = ScriptCan::new(0x7E0, 0x7E8);
        let prof = EcuProfile::tacoma_can();
        let mut client = UdsClient::for_profile(&mut dev, 1, &prof);
        client.apply_server_timing(50, 5000);
        assert!(client.p2_ms() >= 1000, "50 ms server P2 must not become the client deadline");
        assert!(client.p2_star_ms() >= 5000);
    }

    #[test]
    fn stale_tester_present_reply_is_skipped() {
        let mut dev = ScriptCan::new(0x7E0, 0x7E8);
        dev.enqueue_rx(&[0x02, 0x7E, 0x00, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA]);
        dev.enqueue_rx(&[0x06, 0x50, 0x03, 0x00, 0x32, 0x01, 0xF4, 0xAA]);
        let prof = EcuProfile::tacoma_can();
        let mut client = UdsClient::for_profile(&mut dev, 1, &prof);
        client.set_timers(200, 200);
        client
            .diagnostic_session_control(&mut dev, 0x03)
            .expect("stale 7E 00 must not fail the session request");
    }
}
