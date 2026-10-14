use crate::definitions::XdfDocument;
use crate::security_config::locate;
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

pub const SUPPORTED_RECIPE_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize)]
pub struct PatchRecipeFile {
    pub version: u32,
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub operations: Vec<PatchOperation>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PatchOperation {
    FlagSet {
        flag_id: String,
        value: bool,
    },
    ConstantSet {
        constant_id: String,
        engineering_value: f64,
    },
    BytesWrite {
        address: String,
        hex: String,
    },
    TableOffsetPercent {
        table_id: String,
        percent: f64,
    },
}

#[derive(Clone, Debug, Default)]
pub struct ByteDiff {
    pub address: u32,
    pub old_byte: u8,
    pub new_byte: u8,
}

#[derive(Clone, Debug)]
pub struct PatchPreview {
    pub recipe_id: String,
    pub recipe_title: String,
    pub diffs: Vec<ByteDiff>,
    pub notes: Vec<String>,
}

#[derive(Clone)]
pub struct PatchRecipe {
    pub file: PatchRecipeFile,
    pub path: PathBuf,
}

impl PatchRecipe {
    pub fn load(path: &Path) -> Result<Self, String> {
        let raw = fs::read_to_string(path).map_err(|e| format!("read {}: {}", path.display(), e))?;
        let ext = path
            .extension()
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_default();
        let file: PatchRecipeFile = if ext == "yaml" || ext == "yml" {
            serde_yaml::from_str(&raw).map_err(|e| format!("YAML parse: {}", e))?
        } else {
            serde_json::from_str(&raw).map_err(|e| format!("JSON parse: {}", e))?
        };
        if file.version != SUPPORTED_RECIPE_VERSION {
            return Err(format!(
                "unsupported recipe version {} (expected {})",
                file.version,
                SUPPORTED_RECIPE_VERSION
            ));
        }
        validate_recipe(&file)?;
        Ok(Self {
            file,
            path: path.to_path_buf(),
        })
    }

    pub fn preview(
        &self,
        rom: &[u8],
        origin: i64,
        xdf: &XdfDocument,
    ) -> Result<PatchPreview, String> {
        let mut scratch = rom.to_vec();
        let mut xdf_scratch = xdf.clone();
        let mut notes = Vec::new();
        apply_recipe_to_buffers(
            &self.file,
            &mut scratch,
            origin,
            &mut xdf_scratch,
            &mut notes,
            true,
        )?;
        let diffs = diff_roms(rom, &scratch, origin);
        Ok(PatchPreview {
            recipe_id: self.file.id.clone(),
            recipe_title: self.file.title.clone(),
            diffs,
            notes,
        })
    }

    pub fn apply(
        &self,
        rom: &mut [u8],
        origin: i64,
        xdf: &mut XdfDocument,
    ) -> Result<Vec<String>, String> {
        let mut notes = Vec::new();
        apply_recipe_to_buffers(&self.file, rom, origin, xdf, &mut notes, true)?;
        Ok(notes)
    }
}

fn validate_recipe(file: &PatchRecipeFile) -> Result<(), String> {
    if file.operations.is_empty() {
        return Err("recipe has no operations".to_string());
    }
    for op in &file.operations {
        match op {
            PatchOperation::BytesWrite { hex, .. } => {
                if hex.len() % 2 != 0 {
                    return Err("bytes_write hex must have even length".to_string());
                }
                if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
                    return Err("bytes_write hex must be ASCII hex".to_string());
                }
            }
            PatchOperation::TableOffsetPercent { percent, .. } => {
                if percent.abs() > 50.0 {
                    return Err("table_offset_percent limited to ±50%".to_string());
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn parse_address(addr: &str) -> Result<u32, String> {
    let s = addr.trim();
    let s = s.strip_prefix("0x").unwrap_or(s);
    u32::from_str_radix(s, 16).map_err(|e| format!("invalid address {}: {}", addr, e))
}

fn parse_hex_bytes(hex: &str) -> Result<Vec<u8>, String> {
    let hex = hex.trim();
    let mut out = Vec::new();
    for i in (0..hex.len()).step_by(2) {
        if i + 2 > hex.len() {
            break;
        }
        let b = u8::from_str_radix(&hex[i..i + 2], 16)
            .map_err(|e| format!("hex byte: {}", e))?;
        out.push(b);
    }
    Ok(out)
}

fn apply_recipe_to_buffers(
    file: &PatchRecipeFile,
    rom: &mut [u8],
    origin: i64,
    xdf: &mut XdfDocument,
    notes: &mut Vec<String>,
    sync_xdf: bool,
) -> Result<(), String> {
    for op in &file.operations {
        match op {
            PatchOperation::FlagSet { flag_id, value } => {
                let idx = xdf
                    .flags
                    .iter()
                    .position(|f| f.id == *flag_id)
                    .ok_or_else(|| format!("unknown flag_id {}", flag_id))?;
                xdf.flags[idx].is_set = *value;
                if sync_xdf {
                    xdf.write_flag_to_rom(rom, origin, &xdf.flags[idx]);
                }
                notes.push(format!("flag {} = {}", flag_id, value));
            }
            PatchOperation::ConstantSet {
                constant_id,
                engineering_value,
            } => {
                let idx = xdf
                    .constants
                    .iter()
                    .position(|c| c.id == *constant_id)
                    .ok_or_else(|| format!("unknown constant_id {}", constant_id))?;
                xdf.constants[idx].value = *engineering_value;
                if sync_xdf {
                    write_constant_to_rom(rom, origin, &xdf.constants[idx]);
                }
                notes.push(format!(
                    "constant {} = {}",
                    constant_id,
                    engineering_value
                ));
            }
            PatchOperation::BytesWrite { address, hex } => {
                let addr = parse_address(address)?;
                let data = parse_hex_bytes(hex)?;
                let start = locate(rom, origin, addr as i64)
                    .ok_or_else(|| format!("address 0x{:X} out of ROM", addr))?;
                if start + data.len() > rom.len() {
                    return Err(format!(
                        "bytes_write 0x{:X}+{} exceeds ROM",
                        addr,
                        data.len()
                    ));
                }
                for (i, b) in data.iter().enumerate() {
                    rom[start + i] = *b;
                }
                notes.push(format!("bytes 0x{:X} len {}", addr, data.len()));
            }
            PatchOperation::TableOffsetPercent { table_id, percent } => {
                let idx = xdf
                    .tables
                    .iter()
                    .position(|t| t.id == *table_id)
                    .ok_or_else(|| format!("unknown table_id {}", table_id))?;
                let factor = 1.0 + (*percent / 100.0);
                for row in &mut xdf.tables[idx].values {
                    for v in row.iter_mut() {
                        *v = (*v * factor * 100.0).round() / 100.0;
                    }
                }
                if sync_xdf {
                    xdf.write_table_to_rom(rom, origin, idx);
                }
                notes.push(format!("table {} offset {}%", table_id, percent));
            }
        }
    }
    Ok(())
}

fn write_constant_to_rom(rom: &mut [u8], origin: i64, constant: &crate::definitions::XdfConstant) {
    if let Some(idx) = locate(rom, origin, constant.address as i64) {
        let raw = constant.equation.invert(constant.value).round() as i64;
        let raw = raw.clamp(0, 255) as u8;
        rom[idx] = raw;
    }
}

pub fn diff_roms(before: &[u8], after: &[u8], origin: i64) -> Vec<ByteDiff> {
    let len = before.len().min(after.len());
    let mut diffs = Vec::new();
    for i in 0..len {
        if before[i] != after[i] {
            diffs.push(ByteDiff {
                address: (origin + i as i64) as u32,
                old_byte: before[i],
                new_byte: after[i],
            });
        }
    }
    diffs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definitions::XdfDocument;

    #[test]
    fn preview_flag_patch() {
        let json = r#"{
            "version": 1,
            "id": "launch-on",
            "title": "Enable launch",
            "operations": [
                { "kind": "flag_set", "flag_id": "flag_launch_control", "value": true }
            ]
        }"#;
        let file: PatchRecipeFile = serde_json::from_str(json).unwrap();
        let recipe = PatchRecipe {
            file,
            path: PathBuf::from("test.json"),
        };
        let xdf = XdfDocument::sample();
        let rom = vec![0u8; 0x20000];
        let preview = recipe.preview(&rom, 0, &xdf).unwrap();
        assert!(!preview.diffs.is_empty() || !preview.notes.is_empty());
    }
}
