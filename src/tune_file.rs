//! `.wbtune` tune files (HP Tuners `.hpt` / VF tune-file equivalent): ROM image + origin,
//! definition reference, vehicle identity and notes in one JSON file.
//!
//! The ROM is base64 with a CRC-32 so a truncated or hand-edited file is refused instead of
//! loaded as a corrupt calibration. The XDF is referenced by path + CRC-32 (not embedded — XDFs
//! are often licensed separately), so a changed definition is detected on open.

use crate::protocols::ChecksumEngine;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const FORMAT: &str = "wideband-tune/1";
pub const EXTENSION: &str = "wbtune";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TuneFile {
    pub format: String,
    pub saved_utc: String,
    pub profile_id: String,
    #[serde(default)]
    pub vin: String,
    #[serde(default)]
    pub cal_id: String,
    #[serde(default)]
    pub notes: String,
    pub rom_origin: i64,
    pub rom_crc32: u32,
    pub rom_base64: String,
    #[serde(default)]
    pub xdf_path: Option<String>,
    #[serde(default)]
    pub xdf_crc32: Option<u32>,
    #[serde(default)]
    pub xdf_title: String,
}

/// What `open` found about the referenced definition.
#[derive(Clone, Debug, PartialEq)]
pub enum XdfStatus {
    /// Same file and contents as when saved.
    Match(PathBuf),
    /// File exists but its contents changed since the tune was saved.
    Changed(PathBuf),
    /// Path missing / never recorded: the user must open the right XDF.
    Missing,
}

pub struct OpenedTune {
    pub tune: TuneFile,
    pub rom: Vec<u8>,
    pub xdf: XdfStatus,
}

pub struct SaveInput<'a> {
    pub profile_id: &'a str,
    pub vin: &'a str,
    pub cal_id: &'a str,
    pub notes: &'a str,
    pub rom: &'a [u8],
    pub rom_origin: i64,
    pub xdf_path: Option<&'a Path>,
    pub xdf_title: &'a str,
}

pub fn save(path: &Path, input: &SaveInput) -> Result<(), String> {
    let xdf_crc32 = input
        .xdf_path
        .and_then(|p| std::fs::read(p).ok())
        .map(|b| ChecksumEngine::crc32(&b));
    let tune = TuneFile {
        format: FORMAT.to_string(),
        saved_utc: chrono::Utc::now().to_rfc3339(),
        profile_id: input.profile_id.to_string(),
        vin: input.vin.to_string(),
        cal_id: input.cal_id.to_string(),
        notes: input.notes.to_string(),
        rom_origin: input.rom_origin,
        rom_crc32: ChecksumEngine::crc32(input.rom),
        rom_base64: base64_encode(input.rom),
        xdf_path: input.xdf_path.map(|p| p.to_string_lossy().to_string()),
        xdf_crc32,
        xdf_title: input.xdf_title.to_string(),
    };
    let text = serde_json::to_string_pretty(&tune).map_err(|e| e.to_string())?;
    // Write-then-rename so a crash mid-save never leaves a half-written tune.
    let tmp = path.with_extension("wbtune.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("write {}: {}", tmp.display(), e))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename to {}: {}", path.display(), e))
}

pub fn open(path: &Path) -> Result<OpenedTune, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let tune: TuneFile = serde_json::from_str(&text).map_err(|e| format!("not a Wideband tune file: {}", e))?;
    if tune.format != FORMAT {
        return Err(format!("unsupported tune format \"{}\" (expected {})", tune.format, FORMAT));
    }
    let rom = base64_decode(&tune.rom_base64)?;
    let crc = ChecksumEngine::crc32(&rom);
    if crc != tune.rom_crc32 {
        return Err(format!(
            "ROM data is corrupt (CRC 0x{:08X}, expected 0x{:08X}); refusing to load",
            crc, tune.rom_crc32
        ));
    }
    let xdf = match tune.xdf_path.as_deref().map(PathBuf::from) {
        Some(p) => match std::fs::read(&p) {
            Ok(bytes) if Some(ChecksumEngine::crc32(&bytes)) == tune.xdf_crc32 => XdfStatus::Match(p),
            Ok(_) => XdfStatus::Changed(p),
            Err(_) => XdfStatus::Missing,
        },
        None => XdfStatus::Missing,
    };
    Ok(OpenedTune { tune, rom, xdf })
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

pub fn base64_decode(text: &str) -> Result<Vec<u8>, String> {
    let mut val = [255u8; 256];
    for (i, c) in B64.iter().enumerate() {
        val[*c as usize] = i as u8;
    }
    let clean: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if clean.len() % 4 != 0 {
        return Err("ROM data length is not valid base64".to_string());
    }
    let mut out = Vec::with_capacity(clean.len() / 4 * 3);
    for quad in clean.chunks(4) {
        let mut n = 0u32;
        let mut pad = 0;
        for &c in quad {
            n <<= 6;
            if c == b'=' {
                pad += 1;
            } else {
                let v = val[c as usize];
                if v == 255 || pad > 0 {
                    return Err("ROM data contains invalid base64".to_string());
                }
                n |= v as u32;
            }
        }
        let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        out.extend_from_slice(&bytes[..3 - pad.min(2)]);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_roundtrip_all_lengths() {
        for n in 0..10usize {
            let data: Vec<u8> = (0..n as u8).map(|i| i.wrapping_mul(37).wrapping_add(200)).collect();
            assert_eq!(base64_decode(&base64_encode(&data)).unwrap(), data, "len {}", n);
        }
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
        assert!(base64_decode("TW!u").is_err());
    }

    fn tmp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("wideband-test-{}", name))
    }

    #[test]
    fn save_open_roundtrip_and_xdf_status() {
        let xdf = tmp("t.xdf");
        std::fs::write(&xdf, "<XDFFORMAT/>").unwrap();
        let rom: Vec<u8> = (0..4096u32).map(|i| (i * 7) as u8).collect();
        let path = tmp("roundtrip.wbtune");
        save(
            &path,
            &SaveInput {
                profile_id: "tacoma-can",
                vin: "5TFRX5GN1HX000000",
                cal_id: "34715000",
                notes: "first pass",
                rom: &rom,
                rom_origin: 0x10000,
                xdf_path: Some(&xdf),
                xdf_title: "Tacoma",
            },
        )
        .unwrap();
        let opened = open(&path).unwrap();
        assert_eq!(opened.rom, rom);
        assert_eq!(opened.tune.rom_origin, 0x10000);
        assert_eq!(opened.tune.notes, "first pass");
        assert_eq!(opened.xdf, XdfStatus::Match(xdf.clone()));

        std::fs::write(&xdf, "<XDFFORMAT>changed</XDFFORMAT>").unwrap();
        assert_eq!(open(&path).unwrap().xdf, XdfStatus::Changed(xdf.clone()));
        std::fs::remove_file(&xdf).unwrap();
        assert_eq!(open(&path).unwrap().xdf, XdfStatus::Missing);
    }

    #[test]
    fn corrupt_rom_is_refused() {
        let path = tmp("corrupt.wbtune");
        save(
            &path,
            &SaveInput {
                profile_id: "p",
                vin: "",
                cal_id: "",
                notes: "",
                rom: &[1, 2, 3, 4, 5, 6],
                rom_origin: 0,
                xdf_path: None,
                xdf_title: "",
            },
        )
        .unwrap();
        let mut t: TuneFile = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        t.rom_base64 = base64_encode(&[1, 2, 3, 4, 5, 7]);
        std::fs::write(&path, serde_json::to_string(&t).unwrap()).unwrap();
        let err = open(&path).err().unwrap();
        assert!(err.contains("corrupt"), "{}", err);
    }
}
