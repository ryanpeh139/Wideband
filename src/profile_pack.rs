use crate::j2534::{ISO14230, ISO15765, J1850VPW};
use crate::pack_paths::discover_profile_roots;
use crate::plugin_host::PluginRegistry;
use crate::protocols::{DiagnosticProtocol, EcuProfile, PluginRefs, VehicleBus};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

pub const SUPPORTED_PACK_VERSION: u32 = 1;

/// Forbidden top-level keys in a profile pack (keys belong in `security-config.json` only).
const FORBIDDEN_ROOT_KEYS: &[&str] = &[
    "mask",
    "key",
    "keyId",
    "key_id",
    "algorithm",
    "densoGenerations",
    "gmKeys",
    "checksums",
    "configured",
    "xor",
    "add",
    "rotate",
    "generation",
];

#[derive(Clone, Debug)]
pub struct ProfileCatalog {
    pub profiles: Vec<EcuProfile>,
    pub load_note: String,
    pub roots: Vec<PathBuf>,
}

impl ProfileCatalog {
    pub fn load_default() -> Self {
        Self::load_from_roots(&discover_profile_roots())
    }

    pub fn discover_directory() -> PathBuf {
        discover_profile_roots()
            .into_iter()
            .find(|p| p.is_dir())
            .unwrap_or_else(|| PathBuf::from("profiles"))
    }

    pub fn load_from_roots(roots: &[PathBuf]) -> Self {
        let mut profiles = Vec::new();
        let mut errors = Vec::new();
        let mut ids = BTreeSet::new();
        let mut used_roots = Vec::new();

        for dir in roots {
            if !dir.is_dir() {
                continue;
            }
            used_roots.push(dir.clone());
            let partial = Self::load_from_directory(dir);
            for profile in partial.profiles {
                if ids.contains(&profile.id) {
                    continue;
                }
                if let Err(e) = PluginRegistry::require_refs(&profile.plugins) {
                    errors.push(format!("profile '{}': {}", profile.id, e));
                    continue;
                }
                ids.insert(profile.id.clone());
                profiles.push(profile);
            }
            if !partial.load_note.is_empty() && partial.load_note.contains("warnings") {
                errors.push(partial.load_note);
            }
        }

        if profiles.is_empty() {
            return Self {
                profiles: EcuProfile::builtin_defaults(),
                load_note: if errors.is_empty() {
                    "No profile packs found; using built-in defaults.".to_string()
                } else {
                    format!(
                        "Profile pack load failed; using built-in defaults. {}",
                        errors.join(" | ")
                    )
                },
                roots: used_roots,
            };
        }

        profiles.sort_by(|a, b| a.name.cmp(&b.name));
        let load_note = if errors.is_empty() {
            format!(
                "Loaded {} profile pack(s) from {} root(s).",
                profiles.len(),
                used_roots.len()
            )
        } else {
            format!(
                "Loaded {} profile pack(s) with warnings: {}",
                profiles.len(),
                errors.join(" | ")
            )
        };

        Self {
            profiles,
            load_note,
            roots: used_roots,
        }
    }

    pub fn load_from_directory(dir: &Path) -> Self {
        let mut profiles = Vec::new();
        let mut errors = Vec::new();
        let mut ids = BTreeSet::new();

        if !dir.is_dir() {
            return Self {
                profiles: EcuProfile::builtin_defaults(),
                load_note: format!(
                    "Profile pack directory not found ({}); using built-in defaults.",
                    dir.display()
                ),
                roots: vec![dir.to_path_buf()],
            };
        }

        let mut paths: Vec<PathBuf> = match fs::read_dir(dir) {
            Ok(entries) => entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().map(|x| x == "json").unwrap_or(false))
                .collect(),
            Err(e) => {
                return Self {
                    profiles: EcuProfile::builtin_defaults(),
                    load_note: format!(
                        "Could not read {}: {}; using built-in defaults.",
                        dir.display(),
                        e
                    ),
                    roots: vec![dir.to_path_buf()],
                };
            }
        };
        paths.sort();

        if paths.is_empty() {
            return Self {
                profiles: Vec::new(),
                load_note: format!("No profile packs in {}.", dir.display()),
                roots: vec![dir.to_path_buf()],
            };
        }

        for path in paths {
            match load_pack_file(&path) {
                Ok(profile) => {
                    if ids.contains(&profile.id) {
                        errors.push(format!(
                            "{}: duplicate profile id '{}'",
                            path.display(),
                            profile.id
                        ));
                        continue;
                    }
                    ids.insert(profile.id.clone());
                    profiles.push(profile);
                }
                Err(e) => errors.push(format!("{}: {}", path.display(), e)),
            }
        }

        if profiles.is_empty() {
            return Self {
                profiles: Vec::new(),
                load_note: if errors.is_empty() {
                    format!("No valid profile packs in {}.", dir.display())
                } else {
                    format!(
                        "Profile pack load failed ({}): {}",
                        dir.display(),
                        errors.join(" | ")
                    )
                },
                roots: vec![dir.to_path_buf()],
            };
        }

        profiles.sort_by(|a, b| a.name.cmp(&b.name));

        let load_note = if errors.is_empty() {
            format!("Loaded {} profile pack(s) from {}.", profiles.len(), dir.display())
        } else {
            format!(
                "Loaded {} profile pack(s) from {} with warnings: {}",
                profiles.len(),
                dir.display(),
                errors.join(" | ")
            )
        };

        Self {
            profiles,
            load_note,
            roots: vec![dir.to_path_buf()],
        }
    }
}

fn load_pack_file(path: &Path) -> Result<EcuProfile, String> {
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let root: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("invalid JSON: {}", e))?;
    reject_embedded_secrets(&root)?;
    let pack: ProfilePackFile =
        serde_json::from_value(root).map_err(|e| format!("schema mismatch: {}", e))?;
    pack.into_profile()
}

fn reject_embedded_secrets(value: &serde_json::Value) -> Result<(), String> {
    match value {
        serde_json::Value::Object(map) => {
            for key in map.keys() {
                let lower = key.to_ascii_lowercase();
                for forbidden in FORBIDDEN_ROOT_KEYS {
                    if lower == forbidden.to_ascii_lowercase() {
                        return Err(format!(
                            "pack must not embed '{}' (use security-config.json refs only)",
                            key
                        ));
                    }
                }
            }
            for child in map.values() {
                if let serde_json::Value::Object(_) = child {
                    // Only scan one level at root for forbidden keys; refs must not contain secrets.
                    if map.contains_key("refs") {
                        continue;
                    }
                }
            }
            if let Some(refs) = map.get("refs") {
                if let serde_json::Value::Object(ref_map) = refs {
                    for (k, v) in ref_map {
                        if let serde_json::Value::String(s) = v {
                            if s.trim().is_empty() {
                                return Err(format!("refs.{} must not be empty", k));
                            }
                        } else {
                            return Err(format!("refs.{} must be a string id", k));
                        }
                    }
                }
            }
            Ok(())
        }
        _ => Err("profile pack root must be a JSON object".to_string()),
    }
}

#[derive(Debug, Deserialize)]
struct ProfilePackFile {
    pack_version: u32,
    id: String,
    name: String,
    platform: String,
    bus: String,
    diagnostic: String,
    bitrate: u32,
    j2534: J2534Section,
    can: CanSection,
    kline: KlineSection,
    memory: MemorySection,
    sessions: SessionSection,
    refs: RefSection,
    #[serde(default)]
    plugins: PluginSection,
}

#[derive(Debug, Deserialize, Default)]
struct PluginSection {
    #[serde(default)]
    checksum: Option<String>,
    #[serde(default)]
    protocol: Option<String>,
    #[serde(default)]
    patch: Option<String>,
    #[serde(default)]
    manager: Option<String>,
}

#[derive(Debug, Deserialize)]
struct J2534Section {
    protocol: String,
    connect_flags: u32,
    extended_id: bool,
}

#[derive(Debug, Deserialize)]
struct CanSection {
    request_id: String,
    response_id: String,
    filter_mask: String,
}

#[derive(Debug, Deserialize)]
struct KlineSection {
    tester: String,
    ecu: String,
}

#[derive(Debug, Deserialize)]
struct MemorySection {
    max_chunk: usize,
    address_bytes: usize,
    size_bytes: usize,
}

#[derive(Debug, Deserialize)]
struct SessionSection {
    programming: String,
    read: String,
}

#[derive(Debug, Deserialize)]
struct RefSection {
    security: String,
    flash_layout: String,
    checksum: String,
}

impl ProfilePackFile {
    fn into_profile(self) -> Result<EcuProfile, String> {
        if self.pack_version != SUPPORTED_PACK_VERSION {
            return Err(format!(
                "unsupported pack_version {} (expected {})",
                self.pack_version,
                SUPPORTED_PACK_VERSION
            ));
        }
        if self.id.trim().is_empty() {
            return Err("id must not be empty".to_string());
        }
        Ok(EcuProfile {
            id: self.id.trim().to_string(),
            name: self.name,
            platform: self.platform,
            bus: parse_bus(&self.bus)?,
            diagnostic: parse_diagnostic(&self.diagnostic)?,
            bitrate: self.bitrate,
            protocol_id: parse_j2534_protocol(&self.j2534.protocol)?,
            connect_flags: self.j2534.connect_flags,
            extended_id: self.j2534.extended_id,
            request_id: parse_u32_hex(&self.can.request_id, "can.request_id")?,
            response_id: parse_u32_hex(&self.can.response_id, "can.response_id")?,
            filter_mask: parse_u32_hex(&self.can.filter_mask, "can.filter_mask")?,
            tester: parse_u8_hex(&self.kline.tester, "kline.tester")?,
            ecu: parse_u8_hex(&self.kline.ecu, "kline.ecu")?,
            max_chunk: self.memory.max_chunk,
            address_bytes: self.memory.address_bytes,
            size_bytes: self.memory.size_bytes,
            programming_session: parse_u8_hex(&self.sessions.programming, "sessions.programming")?,
            read_session: parse_u8_hex(&self.sessions.read, "sessions.read")?,
            security_profile_id: self.refs.security.trim().to_string(),
            flash_layout_profile_id: self.refs.flash_layout.trim().to_string(),
            checksum_profile_id: self.refs.checksum.trim().to_string(),
            plugins: PluginRefs {
                checksum: self.plugins.checksum,
                protocol: self.plugins.protocol,
                patch: self.plugins.patch,
                manager: self.plugins.manager,
            },
        })
    }
}

fn parse_bus(raw: &str) -> Result<VehicleBus, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "can_isotp" | "can" => Ok(VehicleBus::CanIsoTp),
        "kline" => Ok(VehicleBus::KLine),
        "vpw" | "j1850vpw" => Ok(VehicleBus::Vpw),
        other => Err(format!("unknown bus '{}'", other)),
    }
}

fn parse_diagnostic(raw: &str) -> Result<DiagnosticProtocol, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "uds" => Ok(DiagnosticProtocol::Uds),
        "kwp" | "kwp2000" => Ok(DiagnosticProtocol::Kwp),
        other => Err(format!("unknown diagnostic '{}'", other)),
    }
}

fn parse_j2534_protocol(raw: &str) -> Result<u32, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "iso15765" | "can" => Ok(ISO15765),
        "iso14230" | "kline" => Ok(ISO14230),
        "j1850vpw" | "vpw" => Ok(J1850VPW),
        other => Err(format!("unknown j2534 protocol '{}'", other)),
    }
}

fn parse_u32_hex(raw: &str, field: &str) -> Result<u32, String> {
    let s = raw.trim();
    let stripped = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).unwrap_or(s);
    u32::from_str_radix(stripped, 16).map_err(|_| format!("invalid hex for {}", field))
}

fn parse_u8_hex(raw: &str, field: &str) -> Result<u8, String> {
    let value = parse_u32_hex(raw, field)?;
    if value > 0xFF {
        return Err(format!("{} out of range for u8", field));
    }
    Ok(value as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_shipped_profile_packs() {
        let catalog = ProfileCatalog::load_from_directory(Path::new("profiles"));
        assert!(
            catalog.profiles.len() >= 4,
            "expected packs, note: {}",
            catalog.load_note
        );
        let tacoma = catalog
            .profiles
            .iter()
            .find(|p| p.id == "tacoma-can")
            .expect("tacoma-can pack");
        assert_eq!(tacoma.request_id, 0x7E0);
        assert_eq!(tacoma.security_profile_id, "tacoma-can");
    }

    #[test]
    fn rejects_embedded_mask_in_pack() {
        let json = r#"{
          "pack_version": 1,
          "id": "evil",
          "name": "Evil",
          "platform": "X",
          "bus": "can_isotp",
          "diagnostic": "uds",
          "bitrate": 500000,
          "mask": "0xDEADBEEF",
          "j2534": { "protocol": "iso15765", "connect_flags": 0, "extended_id": false },
          "can": { "request_id": "0x7E0", "response_id": "0x7E8", "filter_mask": "0x7FF" },
          "kline": { "tester": "0xF1", "ecu": "0x10" },
          "memory": { "max_chunk": 1024, "address_bytes": 4, "size_bytes": 4 },
          "sessions": { "programming": "0x02", "read": "0x03" },
          "refs": { "security": "evil", "flash_layout": "evil", "checksum": "evil" }
        }"#;
        let root: serde_json::Value = serde_json::from_str(json).unwrap();
        assert!(reject_embedded_secrets(&root).is_err());
    }
}
