//! Checksum correction routines (plug-in registry).
//! Production ranges still come from `security-config.json`; this module owns the algorithms
//! and extension points for Tacoma/Denso and Silverado/Delco-specific routines.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Computes a checksum over `window`, zeroing `store_offset..store_offset+width` inside the window first.
pub fn compute_block(
    algorithm: &str,
    window: &[u8],
    store_offset: usize,
    width: usize,
    big_endian: bool,
) -> Result<Vec<u8>, String> {
    let algo = algorithm.to_lowercase();
    if let Some(routine) = registry().lock().unwrap().get(&algo) {
        return routine.compute(window, store_offset, width, big_endian);
    }
    match algo.as_str() {
        "sum16" => Ok(pack_u32(byte_sum(window, store_offset, width) as u32 & 0xFFFF, 2, big_endian)),
        "sum32" => Ok(pack_u32(byte_sum(window, store_offset, width), 4, big_endian)),
        "crc32" => {
            let cleared = zero_store(window, store_offset, width);
            Ok(pack_u32(crc32_iso_hdlc(&cleared), 4, big_endian))
        }
        "delco_crc32" => Err(
            "Delco CRC32 checksum routine is not configured yet (Silverado profile pack).".to_string(),
        ),
        "denso_block_sum" => Err(
            "Denso block-sum routine requires a profile pack entry (Tacoma bench pack uses sum32/crc32 from security-config).".to_string(),
        ),
        _ => Err(format!("Unsupported checksum algorithm: {}", algorithm)),
    }
}

pub fn algorithm_width(algorithm: &str) -> usize {
    match algorithm.to_lowercase().as_str() {
        "sum16" => 2,
        "sum32" | "crc32" | "delco_crc32" => 4,
        _ => 0,
    }
}

pub trait ChecksumRoutine: Send + Sync {
    fn id(&self) -> &'static str;
    fn compute(
        &self,
        window: &[u8],
        store_offset: usize,
        width: usize,
        big_endian: bool,
    ) -> Result<Vec<u8>, String>;
}

struct BuiltinSum32;

impl ChecksumRoutine for BuiltinSum32 {
    fn id(&self) -> &'static str {
        "sum32"
    }

    fn compute(
        &self,
        window: &[u8],
        store_offset: usize,
        width: usize,
        big_endian: bool,
    ) -> Result<Vec<u8>, String> {
        Ok(pack_u32(byte_sum(window, store_offset, width), 4, big_endian))
    }
}

fn registry() -> &'static Mutex<HashMap<String, Box<dyn ChecksumRoutine>>> {
    static REG: OnceLock<Mutex<HashMap<String, Box<dyn ChecksumRoutine>>>> = OnceLock::new();
    REG.get_or_init(|| {
        let mut map: HashMap<String, Box<dyn ChecksumRoutine>> = HashMap::new();
        map.insert("sum32".to_string(), Box::new(BuiltinSum32));
        Mutex::new(map)
    })
}

pub fn register_routine(routine: Box<dyn ChecksumRoutine>) {
    registry()
        .lock()
        .unwrap()
        .insert(routine.id().to_string(), routine);
}

pub fn crc32_iso_hdlc(data: &[u8]) -> u32 {
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

pub fn byte_sum(data: &[u8], store_offset: usize, width: usize) -> u32 {
    let cleared = zero_store(data, store_offset, width);
    cleared.iter().map(|&b| b as u32).sum()
}

fn zero_store(data: &[u8], store_offset: usize, width: usize) -> Vec<u8> {
    let mut out = data.to_vec();
    for i in 0..width {
        if store_offset + i < out.len() {
            out[store_offset + i] = 0;
        }
    }
    out
}

fn pack_u32(value: u32, width: usize, big_endian: bool) -> Vec<u8> {
    let mut data = vec![0u8; width];
    for i in 0..width {
        let shift = if big_endian {
            (width - 1 - i) * 8
        } else {
            i * 8
        };
        data[i] = ((value >> shift) & 0xFF) as u8;
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sum32_corrector_zeros_store_field() {
        let window = vec![0x10u8, 0x20, 0x30, 0x40, 0xFF, 0xFF, 0xFF, 0xFF];
        let out = compute_block("sum32", &window, 4, 4, true).expect("sum32");
        assert_eq!(out, vec![0x00, 0x00, 0x00, 0xA0]);
    }

    #[test]
    fn delco_stub_is_explicit() {
        let window = vec![0u8; 8];
        assert!(compute_block("delco_crc32", &window, 4, 4, true).is_err());
    }
}
