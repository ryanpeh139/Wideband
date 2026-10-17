use crate::j2534::{PassThruDevice, PassThruMsg};
use std::thread;
use std::time::{Duration, Instant};

/// ISO 15765-2 N_Bs / N_Cr: max gap between flow control / consecutive frames (1000 ms).
/// Applied per frame, not to the whole message, so long `0x23` replies survive IPC latency.
const N_FRAME_TIMEOUT_MS: u32 = 1000;

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
        if device.cable_isotp() {
            // The cable segments and pads the message and handles flow control itself.
            device.write_msgs(self.channel, &[PassThruMsg::new_can(self.config.tx_id, payload)], self.config.timeout_ms)?;
            return Ok(());
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

    fn frame_gap(&self) -> Duration {
        Duration::from_millis(self.config.timeout_ms.max(N_FRAME_TIMEOUT_MS) as u64)
    }

    fn wait_flow_control(&self, device: &mut dyn PassThruDevice) -> Result<(u8, u8, u8), String> {
        let deadline = self.frame_gap();
        let start = Instant::now();
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
        if device.cable_isotp() {
            return self.receive_whole(device);
        }
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
                    return self.receive_consecutive(device, assembled, len, pending);
                }
            }
        }
        Err("No ISO-TP response within timeout".to_string())
    }

    /// Cable-side ISO-TP: each read is one complete, reassembled message.
    fn receive_whole(&self, device: &mut dyn PassThruDevice) -> Result<Vec<u8>, String> {
        let deadline = Duration::from_millis(self.config.timeout_ms as u64);
        let start = Instant::now();
        while start.elapsed() < deadline {
            for msg in device.read_msgs(self.channel, 50)? {
                if msg.read_id() != self.config.rx_id {
                    continue;
                }
                let p = msg.payload();
                if !p.is_empty() {
                    return Ok(p.to_vec());
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
        mut pending: Vec<PassThruMsg>,
    ) -> Result<Vec<u8>, String> {
        let gap = self.frame_gap();
        let fc = vec![0x30, self.config.block_size, self.config.stmin_ms];
        let mut fc_buf = fc.clone();
        while fc_buf.len() < 8 {
            fc_buf.push(self.config.pad_byte);
        }
        device.write_msgs(self.channel, &[PassThruMsg::new_can(self.config.tx_id, &fc_buf)], self.config.timeout_ms)?;

        let mut expected_seq = 1u8;
        let mut last_frame = Instant::now();
        while assembled.len() < total_len && last_frame.elapsed() < gap {
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
                if assembled.len() > total_len {
                    assembled.truncate(total_len);
                }
                expected_seq = (expected_seq + 1) & 0x0F;
                last_frame = Instant::now();
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

#[cfg(test)]
pub(crate) mod mock {
    use super::*;
    use crate::j2534::{PassThruDevice, PassThruMsg};
    use std::collections::VecDeque;

    pub struct ScriptCan {
        pub tx_id: u32,
        pub rx_id: u32,
        pub connected: bool,
        pub rx_script: VecDeque<Vec<u8>>,
        pub tx_captured: Vec<Vec<u8>>,
        /// Behave like a J2534 ISO15765 channel: whole messages, no padding.
        pub cable_isotp: bool,
    }

    impl ScriptCan {
        pub fn new(tx_id: u32, rx_id: u32) -> Self {
            Self {
                tx_id,
                rx_id,
                connected: true,
                rx_script: VecDeque::new(),
                tx_captured: Vec::new(),
                cable_isotp: false,
            }
        }

        pub fn enqueue_rx(&mut self, payload: &[u8]) {
            self.rx_script.push_back(payload.to_vec());
        }
    }

    impl PassThruDevice for ScriptCan {
        fn open(&mut self) -> Result<(), String> {
            self.connected = true;
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
            if !self.connected {
                return Err("disconnected".to_string());
            }
            let mut out = Vec::new();
            if let Some(p) = self.rx_script.pop_front() {
                let mut buf = p;
                while !self.cable_isotp && buf.len() < 8 {
                    buf.push(0xAA);
                }
                out.push(PassThruMsg::new_can(self.rx_id, &buf));
            }
            Ok(out)
        }
        fn write_msgs(&mut self, _channel: u32, msgs: &[PassThruMsg], _timeout_ms: u32) -> Result<u32, String> {
            if !self.connected {
                return Err("disconnected".to_string());
            }
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
            "mock".to_string()
        }
        fn cable_isotp(&self) -> bool {
            self.cable_isotp
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mock::ScriptCan;

    #[test]
    fn cable_isotp_sends_and_receives_whole_messages() {
        let mut dev = ScriptCan::new(0x7E0, 0x7E8);
        dev.cable_isotp = true;
        let long_reply: Vec<u8> = (0..20u8).collect();
        dev.enqueue_rx(&long_reply);
        let link = IsoTpLink::new(1, IsoTpConfig::default());

        let request = [0x22, 0xF1, 0x90, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06];
        link.send(&mut dev, &request).unwrap();
        assert_eq!(dev.tx_captured, vec![request.to_vec()], "no PCI bytes or padding in cable ISO-TP mode");
        assert_eq!(link.receive(&mut dev).unwrap(), long_reply);
    }

    #[test]
    fn isotp_multiframe_receive_reassembles() {
        let mut dev = ScriptCan::new(0x7E0, 0x7E8);
        dev.enqueue_rx(&[0x10, 0x08, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
        dev.enqueue_rx(&[0x21, 0x11, 0x22, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA]);

        let link = IsoTpLink::new(
            1,
            IsoTpConfig {
                tx_id: 0x7E0,
                rx_id: 0x7E8,
                timeout_ms: 500,
                ..IsoTpConfig::default()
            },
        );
        let payload = link.receive(&mut dev).expect("assemble");
        assert_eq!(payload, vec![0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x11, 0x22]);
        assert!(
            dev.tx_captured.iter().any(|f| f[0] == 0x30),
            "flow control frame must be transmitted"
        );
    }

    /// Mimics J2534Host IPC: one frame per READ round trip, each round trip slow.
    struct SlowCan(ScriptCan);

    impl PassThruDevice for SlowCan {
        fn open(&mut self) -> Result<(), String> {
            self.0.open()
        }
        fn close(&mut self) -> Result<(), String> {
            self.0.close()
        }
        fn connect(&mut self, p: u32, f: u32, b: u32) -> Result<u32, String> {
            self.0.connect(p, f, b)
        }
        fn disconnect(&mut self, ch: u32) -> Result<(), String> {
            self.0.disconnect(ch)
        }
        fn read_msgs(&mut self, ch: u32, t: u32) -> Result<Vec<PassThruMsg>, String> {
            std::thread::sleep(Duration::from_millis(20));
            self.0.read_msgs(ch, t)
        }
        fn write_msgs(&mut self, ch: u32, m: &[PassThruMsg], t: u32) -> Result<u32, String> {
            self.0.write_msgs(ch, m, t)
        }
        fn start_filter(
            &mut self,
            ch: u32,
            ft: u32,
            mask: &PassThruMsg,
            pat: &PassThruMsg,
            fc: Option<&PassThruMsg>,
        ) -> Result<u32, String> {
            self.0.start_filter(ch, ft, mask, pat, fc)
        }
        fn ioctl(&mut self, ch: u32, id: u32, input: &[u8]) -> Result<Vec<u8>, String> {
            self.0.ioctl(ch, id, input)
        }
        fn is_connected(&self) -> bool {
            self.0.is_connected()
        }
        fn firmware_version(&self) -> String {
            self.0.firmware_version()
        }
    }

    #[test]
    fn isotp_long_reply_survives_short_p2_over_slow_ipc() {
        // 6 + 7*7 = 55 bytes over 8 frames at 20 ms each (~160 ms) with a 50 ms P2.
        let mut inner = ScriptCan::new(0x7E0, 0x7E8);
        let total = 55u8;
        let mut ff = vec![0x10, total];
        ff.extend((0..6).map(|i| i as u8));
        inner.enqueue_rx(&ff);
        let mut next = 6u8;
        for seq in 1..=7u8 {
            let mut cf = vec![0x20 | seq];
            for _ in 0..7 {
                cf.push(next);
                next += 1;
            }
            inner.enqueue_rx(&cf);
        }
        let mut dev = SlowCan(inner);
        let link = IsoTpLink::new(
            1,
            IsoTpConfig {
                timeout_ms: 50,
                ..IsoTpConfig::default()
            },
        );
        let payload = link.receive(&mut dev).expect("per-frame timeout, not whole-message");
        assert_eq!(payload, (0..total).collect::<Vec<u8>>());
    }

    #[test]
    fn isotp_multiframe_send_waits_for_flow_control() {
        let mut dev = ScriptCan::new(0x7E0, 0x7E8);
        dev.enqueue_rx(&[0x30, 0x00, 0x00, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA]);

        let link = IsoTpLink::new(
            1,
            IsoTpConfig {
                tx_id: 0x7E0,
                rx_id: 0x7E8,
                timeout_ms: 500,
                ..IsoTpConfig::default()
            },
        );
        let body: Vec<u8> = (0u8..12).collect();
        link.send(&mut dev, &body).expect("send");
        assert!(dev.tx_captured.first().map(|f| f[0] >> 4 == 0x01).unwrap_or(false));
        assert!(dev.tx_captured.iter().any(|f| f[0] >> 4 == 0x02));
    }
}
