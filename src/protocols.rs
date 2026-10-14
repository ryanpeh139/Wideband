use crate::isotp::{IsoTpConfig, IsoTpLink as IsoTpSession};
use crate::j2534::{PassThruDevice, ISO14230, ISO15765, J1850VPW};
use crate::uds::{ProgrammingSession, UdsClient};

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
// ISO-TP (delegates to `isotp` module; legacy static helpers for telemetry)
// ---------------------------------------------------------------------------

pub struct IsoTpLink;

impl IsoTpLink {
    pub fn send(device: &mut dyn PassThruDevice, channel: u32, can_id: u32, payload: &[u8]) -> Result<(), String> {
        let link = IsoTpSession::new(channel, IsoTpConfig {
            tx_id: can_id,
            rx_id: can_id + 8,
            ..IsoTpConfig::default()
        });
        link.send(device, payload)
    }

    pub fn receive(device: &mut dyn PassThruDevice, channel: u32, timeout_ms: u32) -> Result<Vec<u8>, String> {
        let mut link = IsoTpSession::new(channel, IsoTpConfig {
            rx_id: 0x7E8,
            timeout_ms,
            ..IsoTpConfig::default()
        });
        link.receive(device)
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
    /// When `allow_demo_defaults` is true (Virtual ECU bench only), missing fields are filled with demo values.
    pub fn query(
        device: &mut dyn PassThruDevice,
        channel: u32,
        profile: &EcuProfile,
        allow_demo_defaults: bool,
    ) -> Result<EcuIdentification, String> {
        let mut id = EcuIdentification::default();
        let mut field_errors: Vec<String> = Vec::new();

        if profile.bus != VehicleBus::CanIsoTp {
            return Err(format!(
                "ECU identify over {} is not implemented yet — connect a CAN ISO-TP profile.",
                profile.name
            ));
        }

        let mut client = UdsClient::for_profile(device, channel, profile);

        // 1. VIN: OBD Mode 09 02 or UDS 22 F1 90
        let vin_req = if profile.diagnostic == DiagnosticProtocol::Kwp {
            vec![0x09, 0x02]
        } else {
            vec![0x22, 0xF1, 0x90]
        };
        let vin_positive = if profile.diagnostic == DiagnosticProtocol::Kwp {
            0x49
        } else {
            0x62
        };
        match client.request(device, &vin_req, vin_positive) {
            Ok(resp) if resp.len() >= 4 => {
                id.vin = String::from_utf8_lossy(&resp[3..]).trim().to_string();
            }
            Ok(_) => field_errors.push("VIN: ECU response too short".to_string()),
            Err(e) => field_errors.push(format!("VIN: {}", e)),
        }

        // 2. Cal ID
        let cal_req = if profile.diagnostic == DiagnosticProtocol::Kwp {
            vec![0x09, 0x04]
        } else if profile.platform.contains("Toyota") {
            vec![0x22, 0xF1, 0x81]
        } else {
            vec![0x22, 0x00, 0xC1]
        };
        let cal_positive = if profile.diagnostic == DiagnosticProtocol::Kwp {
            0x49
        } else {
            0x62
        };
        match client.request(device, &cal_req, cal_positive) {
            Ok(resp) if resp.len() >= 4 => {
                id.cal_id = String::from_utf8_lossy(&resp[3..]).trim().to_string();
            }
            Ok(_) => field_errors.push("Calibration ID: ECU response too short".to_string()),
            Err(e) => field_errors.push(format!("Calibration ID: {}", e)),
        }

        // 3. CVN (OBD Mode 09 06)
        let cvn_req = vec![0x09, 0x06];
        match client.request(device, &cvn_req, 0x49) {
            Ok(resp) if resp.len() >= 4 => {
                id.cvn = resp[3..]
                    .iter()
                    .map(|b| format!("{:02X}", b))
                    .collect::<Vec<_>>()
                    .join("");
            }
            Ok(_) => field_errors.push("CVN: ECU response too short".to_string()),
            Err(e) => field_errors.push(format!("CVN: {}", e)),
        }

        if allow_demo_defaults {
            if id.vin.is_empty() {
                id.vin = "5TFRX54N56Z123456".to_string();
            }
            if id.cal_id.is_empty() {
                id.cal_id = if profile.platform.contains("Toyota") {
                    "89663-04B04".to_string()
                } else {
                    "12619623".to_string()
                };
            }
            if id.cvn.is_empty() {
                id.cvn = "A1B2C3D4".to_string();
            }
        } else if !field_errors.is_empty() {
            return Err(field_errors.join("; "));
        } else if id.vin.is_empty() && id.cal_id.is_empty() && id.cvn.is_empty() {
            return Err(
                "ECU did not respond to identification (ignition ON, Tacoma 2009+ CAN profile, and bus traffic required)."
                    .to_string(),
            );
        }

        id.details = format!(
            "ECU Identification: VIN: {}, Cal ID: {}, CVN: {} [{}]",
            if id.vin.is_empty() { "(none)" } else { &id.vin },
            if id.cal_id.is_empty() { "(none)" } else { &id.cal_id },
            if id.cvn.is_empty() { "(none)" } else { &id.cvn },
            profile.name
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
        mut progress: impl FnMut(usize, usize),
    ) -> Result<(), String> {
        if image.is_empty() {
            return Err("Download image is empty".to_string());
        }
        let mut client = UdsClient::for_profile(device, channel, profile);
        ProgrammingSession::unlock(&mut client, device, profile)?;
        Self::execute_with_client(&mut client, device, channel, profile, address, image, progress)
    }

    pub fn execute_with_client(
        client: &mut UdsClient,
        device: &mut dyn PassThruDevice,
        _channel: u32,
        profile: &EcuProfile,
        address: u32,
        image: &[u8],
        mut progress: impl FnMut(usize, usize),
    ) -> Result<(), String> {
        let mut dl_req = vec![0x34];
        if profile.diagnostic == DiagnosticProtocol::Kwp {
            dl_req.push(((address >> 16) & 0xFF) as u8);
            dl_req.push(((address >> 8) & 0xFF) as u8);
            dl_req.push((address & 0xFF) as u8);
            dl_req.push(0x00);
            let size = image.len() as u32;
            dl_req.push(((size >> 16) & 0xFF) as u8);
            dl_req.push(((size >> 8) & 0xFF) as u8);
            dl_req.push((size & 0xFF) as u8);
        } else {
            dl_req.push(0x00);
            dl_req.push(0x44);
            dl_req.extend_from_slice(&address.to_be_bytes());
            dl_req.extend_from_slice(&(image.len() as u32).to_be_bytes());
        }

        let _dl_resp = client.request(device, &dl_req, 0x74)?;

        let chunk_size = profile.max_chunk.min(1024);
        let total = image.len();
        let mut offset = 0;
        let mut block_seq = 1u8;

        while offset < total {
            let this_chunk = (total - offset).min(chunk_size);
            let mut tr_req = vec![0x36, block_seq];
            tr_req.extend_from_slice(&image[offset..offset + this_chunk]);

            let tr_resp = client.request(device, &tr_req, 0x76)?;
            if tr_resp.len() < 2 || tr_resp[1] != block_seq {
                return Err(format!("TransferData block {} sequence mismatch", block_seq));
            }

            offset += this_chunk;
            block_seq = block_seq.wrapping_add(1);
            progress(offset, total);
        }

        client.request(device, &[0x37], 0x77)?;
        let _ = client.request(device, &[0x11, 0x01], 0x51);
        Ok(())
    }
}
