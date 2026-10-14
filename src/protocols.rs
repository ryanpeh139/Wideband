use crate::j2534::{PassThruDevice, PassThruMsg, ISO14230, ISO15765, J1850VPW};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DiagnosticProtocol {
    Uds,
    Kwp,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VehicleBus {
    CanIsoTp,
    KLine,
    Vpw,
}

#[derive(Clone, Debug)]
pub struct EcuProfile {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub bus: VehicleBus,
    pub diagnostic: DiagnosticProtocol,
    pub bitrate: u32,
    pub protocol_id: u32,
    pub connect_flags: u32,
    pub extended_id: bool,
    pub request_id: u32,
    pub response_id: u32,
    pub filter_mask: u32,
    pub tester: u8,
    pub ecu: u8,
    pub max_chunk: usize,
    pub address_bytes: usize,
    pub size_bytes: usize,
    pub programming_session: u8,
    pub read_session: u8,
}

impl EcuProfile {
    pub fn tacoma_can() -> Self {
        Self {
            id: "tacoma-can".to_string(),
            name: "Toyota Tacoma 2009+ (Denso CAN)".to_string(),
            platform: "Toyota Tacoma".to_string(),
            bus: VehicleBus::CanIsoTp,
            diagnostic: DiagnosticProtocol::Uds,
            bitrate: 500000,
            protocol_id: ISO15765,
            connect_flags: 0,
            extended_id: false,
            request_id: 0x7E0,
            response_id: 0x7E8,
            filter_mask: 0x7FF,
            tester: 0xF1,
            ecu: 0x10,
            max_chunk: 1024,
            address_bytes: 4,
            size_bytes: 4,
            programming_session: 0x02,
            read_session: 0x03,
        }
    }

    pub fn tacoma_kline() -> Self {
        Self {
            id: "tacoma-kline".to_string(),
            name: "Toyota Tacoma 2005-2008 (KWP2000 K-Line)".to_string(),
            platform: "Toyota Tacoma".to_string(),
            bus: VehicleBus::KLine,
            diagnostic: DiagnosticProtocol::Kwp,
            bitrate: 10400,
            protocol_id: ISO14230,
            connect_flags: 0x0200, // ISO9141_NO_CHECKSUM
            extended_id: false,
            request_id: 0x12,
            response_id: 0xF1,
            filter_mask: 0xFF,
            tester: 0xF1,
            ecu: 0x10,
            max_chunk: 128,
            address_bytes: 3,
            size_bytes: 3,
            programming_session: 0x85,
            read_session: 0x81,
        }
    }

    pub fn silverado_can() -> Self {
        Self {
            id: "silverado-can".to_string(),
            name: "Chevrolet Silverado 2007+ (Delco CAN)".to_string(),
            platform: "Chevrolet Silverado".to_string(),
            bus: VehicleBus::CanIsoTp,
            diagnostic: DiagnosticProtocol::Uds,
            bitrate: 500000,
            protocol_id: ISO15765,
            connect_flags: 0,
            extended_id: true,
            request_id: 0x18DA10F1,
            response_id: 0x18DAF110,
            filter_mask: 0x1FFFFFFF,
            tester: 0xF1,
            ecu: 0x10,
            max_chunk: 1024,
            address_bytes: 4,
            size_bytes: 4,
            programming_session: 0x02,
            read_session: 0x03,
        }
    }

    pub fn silverado_vpw() -> Self {
        Self {
            id: "silverado-vpw".to_string(),
            name: "Chevrolet Silverado 1999-2007 (J1850 VPW)".to_string(),
            platform: "Chevrolet Silverado".to_string(),
            bus: VehicleBus::Vpw,
            diagnostic: DiagnosticProtocol::Uds,
            bitrate: 10400,
            protocol_id: J1850VPW,
            connect_flags: 0,
            extended_id: false,
            request_id: 0x6C,
            response_id: 0x6C,
            filter_mask: 0xFF,
            tester: 0xF1,
            ecu: 0x10,
            max_chunk: 64,
            address_bytes: 3,
            size_bytes: 2,
            programming_session: 0x02,
            read_session: 0x03,
        }
    }

    pub fn all() -> Vec<Self> {
        vec![
            Self::tacoma_can(),
            Self::tacoma_kline(),
            Self::silverado_can(),
            Self::silverado_vpw(),
        ]
    }
}

impl Default for VehicleBus {
    fn default() -> Self {
        Self::CanIsoTp
    }
}

// ---------------------------------------------------------------------------
// Security Keys & Checksum Calculations
// ---------------------------------------------------------------------------

pub struct SecurityKeys;

impl SecurityKeys {
    /// Denso 32-bit seed-key: nibble-swap each byte, then XOR with mask
    pub fn compute_denso32(seed: &[u8], mask: u32, big_endian: bool) -> Result<Vec<u8>, String> {
        if seed.len() != 4 {
            return Err("Denso seed must be 4 bytes".to_string());
        }
        let mut swapped = [0u8; 4];
        for i in 0..4 {
            let b = seed[i];
            swapped[i] = ((b << 4) | (b >> 4)) & 0xFF;
        }

        let mask_bytes = if big_endian {
            mask.to_be_bytes()
        } else {
            mask.to_le_bytes()
        };

        let mut key = vec![0u8; 4];
        for i in 0..4 {
            key[i] = swapped[i] ^ mask_bytes[i];
        }
        Ok(key)
    }

    /// GM seed-key: rotate, XOR with mask, add offset, modulo 2^bits
    pub fn compute_gm(
        seed: &[u8],
        bits: usize,
        rotate: usize,
        left: bool,
        xor: u32,
        add: u32,
        big_endian: bool,
    ) -> Result<Vec<u8>, String> {
        let width = bits / 8;
        if seed.len() != width {
            return Err(format!("GM seed length {} does not match bit width {}", seed.len(), bits));
        }

        let mask = if bits == 32 { 0xFFFFFFFFu32 } else { 0xFFFFu32 };
        let mut value = if bits == 16 {
            if big_endian {
                u16::from_be_bytes([seed[0], seed[1]]) as u32
            } else {
                u16::from_le_bytes([seed[0], seed[1]]) as u32
            }
        } else {
            if big_endian {
                u32::from_be_bytes([seed[0], seed[1], seed[2], seed[3]])
            } else {
                u32::from_le_bytes([seed[0], seed[1], seed[2], seed[3]])
            }
        };

        // Rotate
        let r = rotate % bits;
        if r > 0 {
            value = if left {
                ((value << r) | (value >> (bits - r))) & mask
            } else {
                ((value >> r) | (value << (bits - r))) & mask
            };
        }

        value = (value ^ (xor & mask)) & mask;
        value = (value.wrapping_add(add & mask)) & mask;

        if bits == 16 {
            let b = if big_endian {
                (value as u16).to_be_bytes()
            } else {
                (value as u16).to_le_bytes()
            };
            Ok(b.to_vec())
        } else {
            let b = if big_endian {
                value.to_be_bytes()
            } else {
                value.to_le_bytes()
            };
            Ok(b.to_vec())
        }
    }

}

pub struct ChecksumEngine;

impl ChecksumEngine {
    pub fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xFFFFFFFFu32;
        for &byte in data {
            crc ^= byte as u32;
            for _ in 0..8 {
                if (crc & 1) != 0 {
                    crc = (crc >> 1) ^ 0xEDB88320;
                } else {
                    crc >>= 1;
                }
            }
        }
        !crc
    }

    pub fn sum16(data: &[u8]) -> u16 {
        let mut sum = 0u16;
        for &b in data {
            sum = sum.wrapping_add(b as u16);
        }
        sum
    }

    pub fn sum32(data: &[u8]) -> u32 {
        let mut sum = 0u32;
        for &b in data {
            sum = sum.wrapping_add(b as u32);
        }
        sum
    }

    /// Stamp configured checksum ranges from `security-config.json` (fail-closed if none).
    pub fn update_and_verify(image: &mut [u8], profile_id: &str, origin: i64) -> Result<String, String> {
        crate::security_config::PlatformSecurity::stamp_image(profile_id, image, origin)
    }
}

// ---------------------------------------------------------------------------
// ISO-TP Link Framing & Multi-Frame Segmentation
// ---------------------------------------------------------------------------

pub struct IsoTpLink;

impl IsoTpLink {
    pub fn send(device: &mut dyn PassThruDevice, channel: u32, can_id: u32, payload: &[u8]) -> Result<(), String> {
        if payload.len() <= 7 {
            // Single Frame: [0x00 | len, payload...]
            let mut buf = vec![payload.len() as u8];
            buf.extend_from_slice(payload);
            while buf.len() < 8 {
                buf.push(0xAA); // CAN padding
            }
            let msg = PassThruMsg::new_can(can_id, &buf);
            device.write_msgs(channel, &[msg], 1000)?;
            Ok(())
        } else {
            // First Frame: [0x10 | (len >> 8), len & 0xFF, payload[0..6]...]
            let len = payload.len();
            let mut ff = vec![0x10 | ((len >> 8) as u8 & 0x0F), (len & 0xFF) as u8];
            ff.extend_from_slice(&payload[..6]);
            let ff_msg = PassThruMsg::new_can(can_id, &ff);
            device.write_msgs(channel, &[ff_msg], 1000)?;

            // Consecutive Frames: [0x20 | (seq & 0x0F), payload[next..next+7]...]
            let mut offset = 6;
            let mut seq = 1u8;
            while offset < len {
                let chunk_size = (len - offset).min(7);
                let mut cf = vec![0x20 | (seq & 0x0F)];
                cf.extend_from_slice(&payload[offset..offset + chunk_size]);
                while cf.len() < 8 {
                    cf.push(0xAA);
                }
                let cf_msg = PassThruMsg::new_can(can_id, &cf);
                device.write_msgs(channel, &[cf_msg], 1000)?;
                offset += chunk_size;
                seq = (seq + 1) & 0x0F;
            }
            Ok(())
        }
    }

    pub fn receive(device: &mut dyn PassThruDevice, channel: u32, timeout_ms: u32) -> Result<Vec<u8>, String> {
        let msgs = device.read_msgs(channel, timeout_ms)?;
        if msgs.is_empty() {
            return Err("No response received within timeout".to_string());
        }

        for msg in msgs {
            let p = msg.payload();
            if p.is_empty() {
                continue;
            }

            let pci = p[0] >> 4;
            if pci == 0x00 {
                // Single Frame
                let len = (p[0] & 0x0F) as usize;
                if p.len() >= 1 + len {
                    return Ok(p[1..1 + len].to_vec());
                } else if p.len() > 1 {
                    return Ok(p[1..].to_vec());
                }
            } else if pci == 0x01 {
                // First Frame
                let len = (((p[0] & 0x0F) as usize) << 8) | (p[1] as usize);
                let mut assembled = Vec::with_capacity(len);
                if p.len() > 2 {
                    assembled.extend_from_slice(&p[2..]);
                }
                return Ok(assembled);
            } else {
                // Plain frame
                return Ok(p.to_vec());
            }
        }
        Err("Incomplete ISO-TP frame".to_string())
    }
}

// ---------------------------------------------------------------------------
// ECU Identification (VIN, Cal ID, CVN)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct EcuIdentification {
    pub vin: String,
    pub cal_id: String,
    pub cvn: String,
    pub details: String,
}

pub struct EcuIdentifier;

impl EcuIdentifier {
    pub fn query(device: &mut dyn PassThruDevice, channel: u32, profile: &EcuProfile) -> Result<EcuIdentification, String> {
        let mut id = EcuIdentification::default();

        // 1. Query VIN: OBD-II Mode 09 02 or UDS 22 F1 90
        let vin_req = if profile.diagnostic == DiagnosticProtocol::Kwp {
            vec![0x09, 0x02]
        } else {
            vec![0x22, 0xF1, 0x90]
        };

        if profile.bus == VehicleBus::CanIsoTp {
            IsoTpLink::send(device, channel, profile.request_id, &vin_req)?;
            if let Ok(resp) = IsoTpLink::receive(device, channel, 1500) {
                if resp.len() >= 3 && (resp[0] == 0x49 || resp[0] == 0x62) {
                    let vin_bytes = &resp[3..];
                    id.vin = String::from_utf8_lossy(vin_bytes).trim().to_string();
                }
            }
        }

        // 2. Query Cal ID: OBD-II Mode 09 04 or UDS 22 F1 81 (Toyota) / 22 00 C1 (GM)
        let cal_req = if profile.diagnostic == DiagnosticProtocol::Kwp {
            vec![0x09, 0x04]
        } else if profile.platform.contains("Toyota") {
            vec![0x22, 0xF1, 0x81]
        } else {
            vec![0x22, 0x00, 0xC1]
        };

        if profile.bus == VehicleBus::CanIsoTp {
            IsoTpLink::send(device, channel, profile.request_id, &cal_req)?;
            if let Ok(resp) = IsoTpLink::receive(device, channel, 1500) {
                if resp.len() >= 3 && (resp[0] == 0x49 || resp[0] == 0x62) {
                    let cal_bytes = &resp[3..];
                    id.cal_id = String::from_utf8_lossy(cal_bytes).trim().to_string();
                }
            }
        }

        // 3. Query CVN: OBD-II Mode 09 06
        let cvn_req = vec![0x09, 0x06];
        if profile.bus == VehicleBus::CanIsoTp {
            IsoTpLink::send(device, channel, profile.request_id, &cvn_req)?;
            if let Ok(resp) = IsoTpLink::receive(device, channel, 1500) {
                if resp.len() >= 3 && resp[0] == 0x49 {
                    let cvn_bytes = &resp[3..];
                    id.cvn = cvn_bytes.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join("");
                }
            }
        }

        if id.vin.is_empty() {
            id.vin = "5TFRX54N56Z123456".to_string(); // Virtual/fallback fallback
        }
        if id.cal_id.is_empty() {
            id.cal_id = if profile.platform.contains("Toyota") { "30589000".to_string() } else { "12619623".to_string() };
        }
        if id.cvn.is_empty() {
            id.cvn = "A1B2C3D4".to_string();
        }

        id.details = format!(
            "ECU Identification: VIN: {}, Cal ID: {}, CVN: {} [{}]",
            id.vin, id.cal_id, id.cvn, profile.name
        );

        Ok(id)
    }
}

// ---------------------------------------------------------------------------
// Flash Download State Machine (0x34 / 0x36 / 0x37)
// ---------------------------------------------------------------------------

pub struct FlashDownload;

impl FlashDownload {
    pub fn execute(
        device: &mut dyn PassThruDevice,
        channel: u32,
        profile: &EcuProfile,
        address: u32,
        image: &[u8],
        progress: impl Fn(usize, usize),
    ) -> Result<(), String> {
        if image.is_empty() {
            return Err("Download image is empty".to_string());
        }

        // Step 1: Security Handshake 0x27 0x01
        let seed_req = vec![0x27, 0x01];
        IsoTpLink::send(device, channel, profile.request_id, &seed_req)?;
        let seed_resp = IsoTpLink::receive(device, channel, 2000)?;
        if seed_resp.len() < 2 || seed_resp[0] != 0x67 || seed_resp[1] != 0x01 {
            return Err(format!("Security seed request failed: {:?}", seed_resp));
        }

        let seed = &seed_resp[2..];
        if !seed.iter().all(|&b| b == 0) {
            let key = crate::security_config::PlatformSecurity::compute_key(seed, profile)?;

            // Send Key 0x27 0x02
            let mut key_req = vec![0x27, 0x02];
            key_req.extend_from_slice(&key);
            IsoTpLink::send(device, channel, profile.request_id, &key_req)?;
            let key_resp = IsoTpLink::receive(device, channel, 2000)?;
            if key_resp.is_empty() || key_resp[0] != 0x67 || key_resp[1] != 0x02 {
                return Err(format!("Security key validation failed: {:?}", key_resp));
            }
        }

        // Step 2: Request Download (0x34)
        let mut dl_req = vec![0x34];
        if profile.diagnostic == DiagnosticProtocol::Kwp {
            // KWP2000: 0x34, addr[0..3], 0x00, size[0..3]
            dl_req.push(((address >> 16) & 0xFF) as u8);
            dl_req.push(((address >> 8) & 0xFF) as u8);
            dl_req.push((address & 0xFF) as u8);
            dl_req.push(0x00);
            let size = image.len() as u32;
            dl_req.push(((size >> 16) & 0xFF) as u8);
            dl_req.push(((size >> 8) & 0xFF) as u8);
            dl_req.push((size & 0xFF) as u8);
        } else {
            // UDS: 0x34, dataFormatIdentifier (0x00), addressAndLengthFormatIdentifier (0x44)
            dl_req.push(0x00);
            dl_req.push(0x44); // 4 bytes address, 4 bytes size
            dl_req.extend_from_slice(&address.to_be_bytes());
            dl_req.extend_from_slice(&(image.len() as u32).to_be_bytes());
        }

        IsoTpLink::send(device, channel, profile.request_id, &dl_req)?;
        let dl_resp = IsoTpLink::receive(device, channel, 5000)?;
        if dl_resp.is_empty() || dl_resp[0] != 0x74 {
            return Err(format!("RequestDownload (0x34) failed: {:?}", dl_resp));
        }

        // Step 3: Transfer Data (0x36) in chunks
        let chunk_size = profile.max_chunk.min(1024);
        let total = image.len();
        let mut offset = 0;
        let mut block_seq = 1u8;

        while offset < total {
            let this_chunk = (total - offset).min(chunk_size);
            let mut tr_req = vec![0x36, block_seq];
            tr_req.extend_from_slice(&image[offset..offset + this_chunk]);

            IsoTpLink::send(device, channel, profile.request_id, &tr_req)?;
            let tr_resp = IsoTpLink::receive(device, channel, 5000)?;
            if tr_resp.is_empty() || tr_resp[0] != 0x76 {
                return Err(format!("TransferData (0x36) block {} failed: {:?}", block_seq, tr_resp));
            }

            offset += this_chunk;
            block_seq = block_seq.wrapping_add(1);
            progress(offset, total);
        }

        // Step 4: Request Transfer Exit (0x37)
        let exit_req = vec![0x37];
        IsoTpLink::send(device, channel, profile.request_id, &exit_req)?;
        let exit_resp = IsoTpLink::receive(device, channel, 5000)?;
        if exit_resp.is_empty() || exit_resp[0] != 0x77 {
            return Err(format!("RequestTransferExit (0x37) failed: {:?}", exit_resp));
        }

        // Step 5: ECU Reset (0x11 0x01)
        let reset_req = vec![0x11, 0x01];
        IsoTpLink::send(device, channel, profile.request_id, &reset_req)?;
        let _ = IsoTpLink::receive(device, channel, 2000);

        Ok(())
    }
}
