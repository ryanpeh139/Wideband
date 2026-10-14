use crate::j2534::{PassThruDevice, PassThruMsg};
use std::thread;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct IsoTpConfig {
    pub tx_id: u32,
    pub rx_id: u32,
    pub pad_byte: u8,
    pub timeout_ms: u32,
    pub block_size: u8,
    pub stmin_ms: u8,
    pub max_flow_waits: u32,
}

impl Default for IsoTpConfig {
    fn default() -> Self {
        Self {
            tx_id: 0x7E0,
            rx_id: 0x7E8,
            pad_byte: 0xAA,
            timeout_ms: 2000,
            block_size: 0,
            stmin_ms: 0,
            max_flow_waits: 5,
        }
    }
}

pub struct IsoTpLink {
    channel: u32,
    config: IsoTpConfig,
}

impl IsoTpLink {
    pub fn new(channel: u32, config: IsoTpConfig) -> Self {
        Self { channel, config }
    }

    pub fn set_timeout_ms(&mut self, timeout_ms: u32) {
        if timeout_ms > 0 {
            self.config.timeout_ms = timeout_ms;
        }
    }

    pub fn send(&self, device: &mut dyn PassThruDevice, payload: &[u8]) -> Result<(), String> {
        if payload.is_empty() {
            return Err("ISO-TP cannot send an empty payload".to_string());
        }
        if payload.len() <= 7 {
            let mut buf = vec![payload.len() as u8];
            buf.extend_from_slice(payload);
            while buf.len() < 8 {
                buf.push(self.config.pad_byte);
            }
            device.write_msgs(self.channel, &[PassThruMsg::new_can(self.config.tx_id, &buf)], self.config.timeout_ms)?;
            return Ok(());
        }

        let len = payload.len();
        let mut ff = vec![0x10 | ((len >> 8) as u8 & 0x0F), (len & 0xFF) as u8];
        ff.extend_from_slice(&payload[..6.min(payload.len())]);
        device.write_msgs(self.channel, &[PassThruMsg::new_can(self.config.tx_id, &ff)], self.config.timeout_ms)?;

        let mut offset = 6;
        let mut seq = 1u8;
        let mut block_budget = 0u8;

        while offset < len {
            if block_budget == 0 {
                let (_flow, bs, stmin) = self.wait_flow_control(device)?;
                block_budget = if bs == 0 { 255 } else { bs };
                if stmin > 0 {
                    thread::sleep(Duration::from_millis(stmin as u64));
                }
            }

            let chunk_size = (len - offset).min(7);
            let mut cf = vec![0x20 | (seq & 0x0F)];
            cf.extend_from_slice(&payload[offset..offset + chunk_size]);
            while cf.len() < 8 {
                cf.push(self.config.pad_byte);
            }
            device.write_msgs(self.channel, &[PassThruMsg::new_can(self.config.tx_id, &cf)], self.config.timeout_ms)?;
            offset += chunk_size;
            seq = (seq + 1) & 0x0F;
            if block_budget > 0 {
                block_budget -= 1;
            }
            if self.config.stmin_ms > 0 {
                thread::sleep(Duration::from_millis(self.config.stmin_ms as u64));
            }
        }
        Ok(())
    }

    fn wait_flow_control(&self, device: &mut dyn PassThruDevice) -> Result<(u8, u8, u8), String> {
        let deadline = Duration::from_millis(self.config.timeout_ms as u64);
        let start = std::time::Instant::now();
        let mut waits = 0u32;
        while start.elapsed() < deadline {
            let msgs = device.read_msgs(self.channel, 50)?;
            for msg in msgs {
                if msg.read_id() != self.config.rx_id {
                    continue;
                }
                let p = msg.payload();
                if p.is_empty() {
                    continue;
                }
                let pci = p[0] >> 4;
                if pci != 0x03 {
                    continue;
                }
                let flow = p[0] & 0x0F;
                let bs = if p.len() > 1 { p[1] } else { 0 };
                let stmin = if p.len() > 2 { p[2] } else { 0 };
                if flow == 0x01 {
                    waits += 1;
                    if waits > self.config.max_flow_waits {
                        return Err("ISO-TP flow-control wait limit reached".to_string());
                    }
                    continue;
                }
                if flow == 0x02 {
                    return Err("ISO-TP receiver overflow".to_string());
                }
                return Ok((flow, bs, decode_stmin(stmin)));
            }
        }
        Err("ISO-TP flow control timed out".to_string())
    }

    pub fn receive(&self, device: &mut dyn PassThruDevice) -> Result<Vec<u8>, String> {
        let deadline = Duration::from_millis(self.config.timeout_ms as u64);
        let start = std::time::Instant::now();
        while start.elapsed() < deadline {
            let msgs = device.read_msgs(self.channel, 50)?;
            let mut iter = msgs.into_iter();
            while let Some(msg) = iter.next() {
                if msg.read_id() != self.config.rx_id {
                    continue;
                }
                let p = msg.payload();
                if p.is_empty() {
                    continue;
                }
                let pci = p[0] >> 4;
                if pci == 0x00 {
                    let len = (p[0] & 0x0F) as usize;
                    if p.len() >= 1 + len {
                        return Ok(p[1..1 + len].to_vec());
                    }
                    return Ok(p[1..].to_vec());
                }
                if pci == 0x01 {
                    let len = (((p[0] & 0x0F) as usize) << 8) | (p[1] as usize);
                    let mut assembled = Vec::with_capacity(len);
                    if p.len() > 2 {
                        let take = (len.min(6)).min(p.len() - 2);
                        assembled.extend_from_slice(&p[2..2 + take]);
                    }
                    let pending: Vec<PassThruMsg> = iter.collect();
                    return self.receive_consecutive(device, assembled, len, start, deadline, pending);
                }
            }
        }
        Err("No ISO-TP response within timeout".to_string())
    }

    fn receive_consecutive(
        &self,
        device: &mut dyn PassThruDevice,
        mut assembled: Vec<u8>,
        total_len: usize,
        start: std::time::Instant,
        deadline: Duration,
        mut pending: Vec<PassThruMsg>,
    ) -> Result<Vec<u8>, String> {
        let fc = vec![0x30, self.config.block_size, self.config.stmin_ms];
        let mut fc_buf = fc.clone();
        while fc_buf.len() < 8 {
            fc_buf.push(self.config.pad_byte);
        }
        device.write_msgs(self.channel, &[PassThruMsg::new_can(self.config.tx_id, &fc_buf)], self.config.timeout_ms)?;

        let mut expected_seq = 1u8;
        while assembled.len() < total_len && start.elapsed() < deadline {
            if pending.is_empty() {
                pending = device.read_msgs(self.channel, 50)?;
            }
            while assembled.len() < total_len && !pending.is_empty() {
                let msg = pending.remove(0);
                if msg.read_id() != self.config.rx_id {
                    continue;
                }
                let p = msg.payload();
                if p.is_empty() {
                    continue;
                }
                let pci = p[0] >> 4;
                if pci != 0x02 {
                    continue;
                }
                let seq = p[0] & 0x0F;
                if seq != expected_seq {
                    return Err(format!("ISO-TP sequence mismatch: expected {}, got {}", expected_seq, seq));
                }
                let need = total_len - assembled.len();
                let take = need.min(7).min(p.len() - 1);
                assembled.extend_from_slice(&p[1..1 + take]);
                expected_seq = (expected_seq + 1) & 0x0F;
            }
            if assembled.len() >= total_len {
                assembled.truncate(total_len);
                return Ok(assembled);
            }
        }
        if assembled.len() >= total_len {
            assembled.truncate(total_len);
            Ok(assembled)
        } else {
            Err("Incomplete ISO-TP multi-frame response".to_string())
        }
    }
}

fn decode_stmin(stmin: u8) -> u8 {
    if stmin <= 0x7F {
        stmin
    } else if stmin >= 0xF1 && stmin <= 0xF9 {
        1
    } else {
        127
    }
}
