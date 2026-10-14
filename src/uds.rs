use crate::isotp::{IsoTpConfig, IsoTpLink};
use crate::j2534::PassThruDevice;
use crate::protocols::{DiagnosticProtocol, EcuProfile};

const NEGATIVE: u8 = 0x7F;
const RESPONSE_PENDING: u8 = 0x78;

pub struct UdsClient {
    link: IsoTpLink,
    max_pending: u32,
    p2_ms: u32,
    p2_star_ms: u32,
}

impl UdsClient {
    pub fn for_profile(device: &mut dyn PassThruDevice, channel: u32, profile: &EcuProfile) -> Self {
        let config = IsoTpConfig {
            tx_id: profile.request_id,
            rx_id: profile.response_id,
            pad_byte: 0xAA,
            timeout_ms: 2000,
            block_size: 0,
            stmin_ms: 0,
            max_flow_waits: 5,
        };
        Self {
            link: IsoTpLink::new(channel, config),
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
        let p2 = if resp.len() >= 4 {
            ((resp[2] as u32) << 8) | (resp[3] as u32)
        } else {
            self.p2_ms
        };
        let p2_star = if resp.len() >= 6 {
            ((resp[4] as u32) << 8) | (resp[5] as u32)
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

pub struct ProgrammingSession;

impl ProgrammingSession {
    pub fn unlock(
        client: &mut UdsClient,
        device: &mut dyn PassThruDevice,
        profile: &EcuProfile,
    ) -> Result<(), String> {
        client.tester_present(device, profile.diagnostic)?;
        let session = profile.programming_session;
        let (p2, p2_star) = client.diagnostic_session_control(device, session)?;
        client.set_timers(p2, p2_star);

        let seed_resp = client.request(device, &[0x27, 0x01], 0x67)?;
        if seed_resp.len() < 2 || seed_resp[1] != 0x01 {
            return Err("Security seed response invalid".to_string());
        }
        let seed = &seed_resp[2..];
        if seed.iter().any(|&b| b != 0) {
            let key = crate::security_config::PlatformSecurity::compute_key(seed, profile)?;
            let mut key_req = vec![0x27, 0x02];
            key_req.extend_from_slice(&key);
            let key_resp = client.request(device, &key_req, 0x67)?;
            if key_resp.len() < 2 || key_resp[1] != 0x02 {
                return Err("Security key rejected".to_string());
            }
        }
        Ok(())
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
        assert_eq!(p2_star, 500);
    }
}
