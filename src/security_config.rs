use crate::protocols::{EcuProfile, SecurityKeys};
// Keys are stored in PlatformState and dispatched here (fail-closed; no embedded production masks).
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

#[cfg(test)]
fn test_serial_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

pub const CHECKSUM_VALID: &str = "Checksum Valid";
pub const CHECKSUM_UPDATED: &str = "Checksum Updated";
pub const CHECKSUM_NOT_CONFIGURED: &str = "Checksum Not Configured";

/// How programming keys were registered (JSON file vs Virtual ECU bench override).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyRegistration {
    None,
    Json,
    Bench,
}

/// High-level security posture for the active cable + config.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecurityMode {
    FailClosed,
    Bench,
    Production,
}

#[derive(Clone, Debug)]
pub struct ProfileSecurityLine {
    pub profile_id: String,
    pub algorithm: String,
    pub configured_in_json: bool,
    pub key_registration: KeyRegistration,
}

#[derive(Clone, Debug)]
pub struct ChecksumSecurityLine {
    pub id: String,
    pub profile: String,
    pub algorithm: String,
    pub configured_in_json: bool,
    pub active: bool,
}

#[derive(Clone, Debug)]
pub struct SecurityReport {
    pub mode: SecurityMode,
    pub config_path: String,
    pub load_note: String,
    pub profiles: Vec<ProfileSecurityLine>,
    pub checksums: Vec<ChecksumSecurityLine>,
    pub flash_blockers: Vec<String>,
}

const BUILT_IN_PROFILES: [&str; 4] = [
    "tacoma-can",
    "tacoma-kline",
    "silverado-can",
    "silverado-vpw",
];

#[derive(Clone, Debug)]
enum ProfileKeyKind {
    Denso {
        mask: u32,
        big_endian: bool,
    },
    Gm {
        bits: usize,
        rotate: usize,
        left: bool,
        xor: u32,
        add: u32,
        big_endian: bool,
    },
}

#[derive(Clone, Debug)]
struct ChecksumEntry {
    profile: String,
    algorithm: String,
    start: i64,
    end: i64,
    store: i64,
    big_endian: bool,
}

struct ProfileDeclaration {
    algorithm: String,
    configured_in_json: bool,
}

struct ChecksumDeclaration {
    id: String,
    profile: String,
    algorithm: String,
    configured_in_json: bool,
    active: bool,
}

struct PlatformState {
    profile_keys: HashMap<String, ProfileKeyKind>,
    /// Profiles whose seed-key mapping was loaded from `security-config.json` (`configured: true`).
    file_configured_profiles: HashSet<String>,
    checksums: Vec<ChecksumEntry>,
    load_note: String,
    key_registration: HashMap<String, KeyRegistration>,
    profile_declarations: HashMap<String, ProfileDeclaration>,
    checksum_declarations: Vec<ChecksumDeclaration>,
    last_config_path: PathBuf,
}

impl PlatformState {
    fn empty() -> Self {
        Self {
            profile_keys: HashMap::new(),
            file_configured_profiles: HashSet::new(),
            checksums: Vec::new(),
            load_note: String::new(),
            key_registration: HashMap::new(),
            profile_declarations: HashMap::new(),
            checksum_declarations: Vec::new(),
            last_config_path: PathBuf::new(),
        }
    }
}

fn state() -> &'static Mutex<PlatformState> {
    static STATE: OnceLock<Mutex<PlatformState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(PlatformState::empty()))
}

pub struct PlatformSecurity;

impl PlatformSecurity {
    /// Hold for the full body of any test that reads or writes global security state (parallel-safe).
    #[cfg(test)]
    pub fn test_serial_guard() -> std::sync::MutexGuard<'static, ()> {
        test_serial_lock()
    }

    pub fn default_config_path() -> PathBuf {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let beside = dir.join("security-config.json");
                if beside.exists() {
                    return beside;
                }
            }
        }
        PathBuf::from("security-config.json")
    }

    pub fn install(path: &Path) {
        let mut guard = state().lock().unwrap();
        guard.profile_keys.clear();
        guard.file_configured_profiles.clear();
        guard.checksums.clear();
        guard.load_note.clear();
        guard.key_registration.clear();
        guard.profile_declarations.clear();
        guard.checksum_declarations.clear();
        guard.last_config_path = path.to_path_buf();
        let result = (|| {
            if path.as_os_str().is_empty() {
                return Err("The security config path is empty.".to_string());
            }
            if !path.exists() {
                fs::write(path, template_json()).map_err(|e| format!("Could not create {}: {}", path.display(), e))?;
            }
            let text = fs::read_to_string(path).map_err(|e| format!("Could not read {}: {}", path.display(), e))?;
            parse_into(&text, &mut guard)
        })();

        if let Err(e) = result {
            guard.profile_keys.clear();
            guard.file_configured_profiles.clear();
            guard.checksums.clear();
            guard.key_registration.clear();
            guard.profile_declarations.clear();
            guard.checksum_declarations.clear();
            guard.load_note = if e.is_empty() {
                "The security config could not be read.".to_string()
            } else {
                e
            };
        } else {
            refresh_key_registration(&mut guard);
        }
    }

    pub fn load_note() -> String {
        state().lock().unwrap().load_note.clone()
    }

    /// True when the profile has a production seed-key entry from `security-config.json` (not virtual-bench overlay).
    pub fn security_key_configured_for_profile(profile_id: &str) -> bool {
        let id = canonical_profile(profile_id);
        state().lock().unwrap().file_configured_profiles.contains(&id)
    }

    /// Bench-only key registration for the virtual ECU loopback (mask 0). Not for production hardware.
    pub fn register_bench_keys_for_virtual() {
        let mut guard = state().lock().unwrap();
        for id in ["tacoma-can", "tacoma-kline"] {
            guard.profile_keys.insert(
                id.to_string(),
                ProfileKeyKind::Denso {
                    mask: 0,
                    big_endian: true,
                },
            );
            guard
                .key_registration
                .insert(id.to_string(), KeyRegistration::Bench);
        }
        guard.profile_keys.insert(
            "silverado-can".to_string(),
            ProfileKeyKind::Gm {
                bits: 32,
                rotate: 0,
                left: true,
                xor: 0,
                add: 0,
                big_endian: true,
            },
        );
        guard
            .key_registration
            .insert("silverado-can".to_string(), KeyRegistration::Bench);
        guard.profile_keys.insert(
            "silverado-vpw".to_string(),
            ProfileKeyKind::Gm {
                bits: 16,
                rotate: 0,
                left: true,
                xor: 0,
                add: 0,
                big_endian: true,
            },
        );
        guard
            .key_registration
            .insert("silverado-vpw".to_string(), KeyRegistration::Bench);
    }

    pub fn security_mode(profile_id: &str, virtual_ecu: bool) -> SecurityMode {
        let id = canonical_profile(profile_id);
        let guard = state().lock().unwrap();
        if !guard.load_note.is_empty() && guard.profile_keys.is_empty() {
            return SecurityMode::FailClosed;
        }
        if virtual_ecu {
            if guard.key_registration.get(&id).is_some() {
                return SecurityMode::Bench;
            }
            return SecurityMode::FailClosed;
        }
        match guard.key_registration.get(&id) {
            Some(KeyRegistration::Json) => SecurityMode::Production,
            _ => SecurityMode::FailClosed,
        }
    }

    pub fn flash_blockers(profile_id: &str, virtual_ecu: bool) -> Vec<String> {
        let id = canonical_profile(profile_id);
        let guard = state().lock().unwrap();
        let mut reasons = Vec::new();
        let bench_virtual = virtual_ecu
            && guard.key_registration.get(&id) == Some(&KeyRegistration::Bench);

        if !guard.load_note.is_empty() && !bench_virtual {
            reasons.push(format!("Security config: {}", guard.load_note));
        }

        match guard.key_registration.get(&id) {
            Some(KeyRegistration::Json) => {}
            Some(KeyRegistration::Bench) if virtual_ecu => {}
            Some(KeyRegistration::Bench) => {
                reasons.push(
                    "Production flash blocked: only Virtual ECU bench keys (zero mask) are active. \
Configure security-config.json with your authorized Denso mask or generation before flashing real hardware."
                        .to_string(),
                );
            }
            Some(KeyRegistration::None) | None => {
                let decl = guard.profile_declarations.get(&id);
                if let Some(d) = decl {
                    if d.configured_in_json {
                        reasons.push(format!(
                            "Profile \"{}\" is marked configured in security-config.json but the key did not load (missing mask, generation, or keyId reference).",
                            id
                        ));
                    } else {
                        reasons.push(format!(
                            "No security key for {}. Set profiles.{}.configured to true with a valid denso32 mask or generation (see tests/security-config-tacoma-can.sample.json).",
                            id, id
                        ));
                    }
                } else {
                    reasons.push(format!(
                        "No security key function is configured for {}.",
                        id
                    ));
                }
            }
        }

        let has_active_checksum = guard
            .checksums
            .iter()
            .any(|e| applies_checksum(e, &id));
        if !has_active_checksum && !bench_virtual {
            let pending = guard
                .checksum_declarations
                .iter()
                .any(|c| c.configured_in_json && profile_matches_checksum(&c.profile, &id));
            if pending {
                reasons.push(format!(
                    "Checksum for \"{}\" is marked configured in JSON but did not load (check start/end/store addresses and algorithm).",
                    id
                ));
            } else {
                reasons.push(format!(
                    "No checksum range is configured for {}. Add a checksums[] row with profile \"{}\" and configured true.",
                    id, id
                ));
            }
        }

        reasons
    }

    pub fn assert_flash_allowed(profile_id: &str, virtual_ecu: bool) -> Result<(), String> {
        let blockers = Self::flash_blockers(profile_id, virtual_ecu);
        if blockers.is_empty() {
            Ok(())
        } else {
            Err(blockers.join(" "))
        }
    }

    pub fn security_report(profile_id: &str, virtual_ecu: bool) -> SecurityReport {
        let mode = Self::security_mode(profile_id, virtual_ecu);
        let flash_blockers = Self::flash_blockers(profile_id, virtual_ecu);
        let guard = state().lock().unwrap();

        let profiles: Vec<ProfileSecurityLine> = BUILT_IN_PROFILES
            .iter()
            .map(|built| {
                let built = built.to_string();
                let decl = guard.profile_declarations.get(&built);
                ProfileSecurityLine {
                    profile_id: built.clone(),
                    algorithm: decl
                        .map(|d| d.algorithm.clone())
                        .unwrap_or_else(|| "—".to_string()),
                    configured_in_json: decl.map(|d| d.configured_in_json).unwrap_or(false),
                    key_registration: guard
                        .key_registration
                        .get(&built)
                        .copied()
                        .unwrap_or(KeyRegistration::None),
                }
            })
            .collect();

        let checksums: Vec<ChecksumSecurityLine> = guard
            .checksum_declarations
            .iter()
            .map(|c| ChecksumSecurityLine {
                id: c.id.clone(),
                profile: c.profile.clone(),
                algorithm: c.algorithm.clone(),
                configured_in_json: c.configured_in_json,
                active: c.active,
            })
            .collect();

        SecurityReport {
            mode,
            config_path: guard.last_config_path.display().to_string(),
            load_note: guard.load_note.clone(),
            profiles,
            checksums,
            flash_blockers,
        }
    }

    pub fn compute_key(seed: &[u8], profile: &EcuProfile) -> Result<Vec<u8>, String> {
        if seed.iter().all(|&b| b == 0) {
            return Ok(vec![0u8; seed.len()]);
        }
        let guard = state().lock().unwrap();
        let id = canonical_profile(&profile.security_profile_id);
        let kind = guard
            .profile_keys
            .get(&id)
            .ok_or_else(|| format!("No security key function is configured for {}.", profile.name))?;
        dispatch_key(seed, kind)
    }

    pub fn handshake_line(seed: &[u8], key: &[u8]) -> String {
        format!("0x27 seed {} key {}", hex_bytes(seed), hex_bytes(key))
    }

    pub fn checksum_configured(profile_id: &str, image: &[u8], origin: i64) -> bool {
        PlatformSecurity::checksum_for(profile_id, image, origin).is_some()
    }

    /// Fail-closed gate before flash: require configured ranges and a verifiable stamp.
    pub fn ensure_flash_checksum(
        profile_id: &str,
        profile_name: &str,
        image: &[u8],
        origin: i64,
    ) -> Result<Vec<u8>, String> {
        if image.is_empty() {
            return Err("The download image is empty.".to_string());
        }
        let name = if profile_name.is_empty() {
            profile_id.to_string()
        } else {
            profile_name.to_string()
        };
        let group = PlatformSecurity::checksum_for(profile_id, image, origin).ok_or_else(|| {
            format!(
                "No checksum range is configured for {}. Flashing edited calibrations is blocked.",
                name
            )
        })?;
        let mut corrected = image.to_vec();
        group.stamp(&mut corrected);
        if !group.verify(&corrected) {
            return Err(
                "Checksum correction failed after stamping. Flashing is blocked.".to_string(),
            );
        }
        Ok(corrected)
    }

    pub fn checksum_for(profile_id: &str, image: &[u8], origin: i64) -> Option<BlockChecksumGroup> {
        if image.is_empty() {
            return None;
        }
        let entries = state().lock().unwrap().checksums.clone();
        let mut blocks = Vec::new();
        for entry in entries {
            if !applies_checksum(&entry, profile_id) {
                continue;
            }
            if let Some((start, length, store)) = try_range(image, origin, &entry) {
                blocks.push(BlockChecksum::new(
                    entry.algorithm.clone(),
                    start,
                    length,
                    store,
                    entry.big_endian,
                ));
            }
        }
        if blocks.is_empty() {
            None
        } else {
            Some(BlockChecksumGroup { blocks })
        }
    }

    pub fn prepare_for_flash(
        profile_id: &str,
        profile_name: &str,
        image: &[u8],
        origin: i64,
    ) -> Result<Vec<u8>, String> {
        PlatformSecurity::ensure_flash_checksum(profile_id, profile_name, image, origin)
    }

    pub fn stamp_image(
        profile_id: &str,
        image: &mut [u8],
        origin: i64,
    ) -> Result<String, String> {
        let before = image.to_vec();
        let group = PlatformSecurity::checksum_for(profile_id, image, origin).ok_or_else(|| {
            format!(
                "No checksum range is configured for {}.",
                canonical_profile(profile_id)
            )
        })?;
        group.stamp(image);
        if !group.verify(image) {
            return Err("The checksum could not be stored.".to_string());
        }
        Ok(mark_checksum(&before, image))
    }

    /// Badge text for the calibration header: valid when stored checksum matches ROM, else updated.
    pub fn checksum_badge(profile_id: &str, image: &[u8], origin: i64) -> String {
        match PlatformSecurity::checksum_for(profile_id, image, origin) {
            Some(group) if group.verify(image) => CHECKSUM_VALID.to_string(),
            Some(_) => CHECKSUM_UPDATED.to_string(),
            None => CHECKSUM_NOT_CONFIGURED.to_string(),
        }
    }
}

pub struct BlockChecksumGroup {
    blocks: Vec<BlockChecksum>,
}

impl BlockChecksumGroup {
    pub fn verify(&self, image: &[u8]) -> bool {
        self.blocks.iter().all(|b| b.verify(image))
    }

    pub fn stamp(&self, image: &mut [u8]) {
        for b in &self.blocks {
            b.stamp(image);
        }
    }
}

pub struct BlockChecksum {
    algorithm: String,
    range_start: usize,
    range_length: usize,
    field_offset: usize,
    big_endian: bool,
}

impl BlockChecksum {
    fn new(
        algorithm: String,
        range_start: usize,
        range_length: usize,
        field_offset: usize,
        big_endian: bool,
    ) -> Self {
        Self {
            algorithm,
            range_start,
            range_length,
            field_offset,
            big_endian,
        }
    }

    pub fn verify(&self, image: &[u8]) -> bool {
        match self.compute(image) {
            Ok(actual) => {
                if self.field_offset + actual.len() > image.len() {
                    return false;
                }
                image[self.field_offset..self.field_offset + actual.len()] == actual[..]
            }
            Err(_) => false,
        }
    }

    pub fn stamp(&self, image: &mut [u8]) {
        let actual = self
            .compute(image)
            .map_err(|e| e.to_string())
            .expect("checksum compute");
        if self.field_offset + actual.len() > image.len() {
            panic!("checksum field out of range");
        }
        image[self.field_offset..self.field_offset + actual.len()].copy_from_slice(&actual);
    }

    fn compute(&self, image: &[u8]) -> Result<Vec<u8>, String> {
        if self.range_start + self.range_length > image.len() {
            return Err("range out of bounds".to_string());
        }
        let window = image[self.range_start..self.range_start + self.range_length].to_vec();
        let width = algorithm_width(&self.algorithm);
        let local = if self.field_offset >= self.range_start {
            self.field_offset - self.range_start
        } else {
            0
        };
        crate::checksum_routine::compute_block(
            &self.algorithm,
            &window,
            local,
            width,
            self.big_endian,
        )
    }
}

fn mark_checksum(before: &[u8], after: &[u8]) -> String {
    if before.len() != after.len() {
        return CHECKSUM_UPDATED.to_string();
    }
    for i in 0..before.len() {
        if before[i] != after[i] {
            return CHECKSUM_UPDATED.to_string();
        }
    }
    CHECKSUM_VALID.to_string()
}

fn applies_checksum(entry: &ChecksumEntry, profile_id: &str) -> bool {
    if entry.profile.is_empty() {
        return true;
    }
    canonical_profile(profile_id).eq_ignore_ascii_case(&entry.profile)
}

fn try_range(image: &[u8], origin: i64, entry: &ChecksumEntry) -> Option<(usize, usize, usize)> {
    let start = locate(image, origin, entry.start)?;
    let end = locate(image, origin, entry.end)?;
    let store = locate(image, origin, entry.store)?;
    let width = algorithm_width(&entry.algorithm);
    if width == 0 || end < start || store + width > image.len() {
        return None;
    }
    let length = end - start + 1;
    if start + length > image.len() {
        return None;
    }
    Some((start, length, store))
}

pub fn locate(image: &[u8], origin: i64, address: i64) -> Option<usize> {
    if address >= 0 && address < image.len() as i64 {
        return Some(address as usize);
    }
    let absolute = address - origin;
    if absolute >= 0 && absolute < image.len() as i64 {
        return Some(absolute as usize);
    }
    None
}

fn algorithm_width(algorithm: &str) -> usize {
    match algorithm {
        "sum16" => 2,
        "sum32" | "crc32" => 4,
        _ => 0,
    }
}

fn hex_bytes(data: &[u8]) -> String {
    data.iter().map(|b| format!("{:02X}", b)).collect()
}

fn canonical_profile(id: &str) -> String {
    let trimmed = id.trim();
    for built in BUILT_IN_PROFILES {
        if built.eq_ignore_ascii_case(trimmed) {
            return built.to_string();
        }
    }
    trimmed.to_string()
}

fn template_json() -> &'static str {
    r#"{
  "profiles": {
    "tacoma-can": { "algorithm": "denso32", "mask": "0x00000000", "generation": "", "endian": "big", "configured": false },
    "tacoma-kline": { "algorithm": "denso32", "mask": "0x00000000", "generation": "", "endian": "big", "configured": false },
    "silverado-can": { "algorithm": "gm", "keyId": "", "configured": false },
    "silverado-vpw": { "algorithm": "gm", "keyId": "", "configured": false }
  },
  "densoGenerations": [ { "id": "example", "mask": "0x00000000", "endian": "big", "configured": false } ],
  "gmKeys": [ { "id": "example", "bits": 16, "rotate": 1, "direction": "left", "xor": "0x0000", "add": 0, "endian": "big", "configured": false } ],
  "checksums": [ { "id": "example", "profile": "", "algorithm": "crc32", "start": "0x0", "end": "0x0", "store": "0x0", "endian": "big", "configured": false } ]
}
"#
}

// --- JSON parsing ---

#[derive(Deserialize)]
struct RootDoc {
    profiles: Option<HashMap<String, ProfileDoc>>,
    #[serde(rename = "densoGenerations")]
    denso_generations: Option<Vec<DensoDoc>>,
    #[serde(rename = "gmKeys")]
    gm_keys: Option<Vec<GmDoc>>,
    checksums: Option<Vec<ChecksumDoc>>,
}

#[derive(Deserialize)]
struct ProfileDoc {
    algorithm: Option<String>,
    mask: Option<String>,
    generation: Option<String>,
    #[serde(rename = "keyId")]
    key_id: Option<String>,
    endian: Option<String>,
    configured: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct DensoDoc {
    id: Option<String>,
    mask: Option<String>,
    endian: Option<String>,
    configured: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct GmDoc {
    id: Option<String>,
    bits: Option<i64>,
    rotate: Option<i64>,
    direction: Option<String>,
    xor: Option<String>,
    add: Option<serde_json::Value>,
    endian: Option<String>,
    configured: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct ChecksumDoc {
    id: Option<String>,
    profile: Option<String>,
    algorithm: Option<String>,
    start: Option<serde_json::Value>,
    end: Option<serde_json::Value>,
    store: Option<serde_json::Value>,
    endian: Option<String>,
    configured: Option<serde_json::Value>,
}

fn parse_into(text: &str, state: &mut PlatformState) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("The security config is empty.".to_string());
    }
    let root: RootDoc = serde_json::from_str(text).map_err(|e| format!("security-config.json parse error: {}", e))?;
    validate_root(&root)?;
    record_profile_declarations(&root, state)?;
    let generations = read_generations(&root)?;
    let gm_keys = read_gm_keys(&root)?;
    let (checksums, checksum_declarations) = read_checksums(&root)?;
    read_profiles(&root, &generations, &gm_keys, state)?;
    state.checksums = checksums;
    state.checksum_declarations = checksum_declarations;
    Ok(())
}

fn validate_root(root: &RootDoc) -> Result<(), String> {
    if root.profiles.is_none() {
        return Err("security-config.json must contain a \"profiles\" object.".to_string());
    }
    let profiles = root.profiles.as_ref().unwrap();
    if profiles.is_empty() {
        return Err("security-config.json \"profiles\" must not be empty.".to_string());
    }
    let mut seen_denso = std::collections::HashSet::new();
    if let Some(rows) = &root.denso_generations {
        for item in rows {
            let id = optional_str(&item.id);
            if id.is_empty() {
                continue;
            }
            if !seen_denso.insert(id.to_lowercase()) {
                return Err(format!("Duplicate densoGenerations id \"{}\".", id));
            }
        }
    }
    let mut seen_gm = std::collections::HashSet::new();
    if let Some(rows) = &root.gm_keys {
        for item in rows {
            let id = optional_str(&item.id);
            if id.is_empty() {
                continue;
            }
            if !seen_gm.insert(id.to_lowercase()) {
                return Err(format!("Duplicate gmKeys id \"{}\".", id));
            }
        }
    }
    Ok(())
}

fn record_profile_declarations(root: &RootDoc, state: &mut PlatformState) -> Result<(), String> {
    state.profile_declarations.clear();
    let profiles = root.profiles.as_ref().unwrap();
    for (key, item) in profiles {
        let id = canonical_profile(key);
        if id.is_empty() {
            return Err("A profile id is required.".to_string());
        }
        let algorithm = optional_str(&item.algorithm);
        if !algorithm.is_empty() {
            let alg = algorithm.to_lowercase();
            if alg != "denso32" && alg != "gm" {
                return Err(format!(
                    "profiles.{} algorithm must be denso32 or gm (got \"{}\").",
                    id,
                    algorithm
                ));
            }
        }
        let configured = flag(&item.configured)?;
        state.profile_declarations.insert(
            id,
            ProfileDeclaration {
                algorithm: if algorithm.is_empty() {
                    "—".to_string()
                } else {
                    algorithm.to_lowercase()
                },
                configured_in_json: configured,
            },
        );
    }
    Ok(())
}

fn refresh_key_registration(state: &mut PlatformState) {
    state.key_registration.clear();
    for id in state.profile_keys.keys() {
        state
            .key_registration
            .insert(id.clone(), KeyRegistration::Json);
    }
}

fn profile_matches_checksum(checksum_profile: &str, profile_id: &str) -> bool {
    if checksum_profile.is_empty() {
        return true;
    }
    canonical_profile(checksum_profile).eq_ignore_ascii_case(&canonical_profile(profile_id))
}

struct DensoGen {
    id: String,
    mask: u32,
    big_endian: bool,
}

struct GmKeyRow {
    id: String,
    bits: usize,
    rotate: usize,
    left: bool,
    xor: u32,
    add: u32,
    big_endian: bool,
}

fn read_generations(root: &RootDoc) -> Result<Vec<DensoGen>, String> {
    let mut list = Vec::new();
    let rows = root.denso_generations.as_ref();
    if rows.is_none() {
        return Ok(list);
    }
    for item in rows.unwrap() {
        if !flag(&item.configured)? {
            continue;
        }
        let id = optional_str(&item.id);
        if id.is_empty() {
            return Err("A Denso generation needs an id.".to_string());
        }
        let mask = mask_from_hex_str(
            item.mask.as_ref().ok_or_else(|| format!("{} needs a mask.", id))?,
            32,
        )?;
        list.push(DensoGen {
            id,
            mask,
            big_endian: big_endian_of(&item.endian)?,
        });
    }
    Ok(list)
}

fn read_gm_keys(root: &RootDoc) -> Result<Vec<GmKeyRow>, String> {
    let mut list = Vec::new();
    let rows = root.gm_keys.as_ref();
    if rows.is_none() {
        return Ok(list);
    }
    for item in rows.unwrap() {
        if !flag(&item.configured)? {
            continue;
        }
        let id = optional_str(&item.id);
        if id.is_empty() {
            return Err("A GM key needs an id.".to_string());
        }
        let bits = item
            .bits
            .ok_or_else(|| format!("{} needs a bit width.", id))? as usize;
        if bits != 16 && bits != 32 {
            return Err(format!("{} must be 16 or 32 bits.", id));
        }
        let rotate = if let Some(r) = item.rotate {
            if r < 0 {
                return Err(format!("{} has a negative rotate count.", id));
            }
            r as usize
        } else {
            0
        };
        let direction = optional_str(&item.direction);
        let left = if direction.is_empty() || direction.eq_ignore_ascii_case("left") {
            true
        } else if direction.eq_ignore_ascii_case("right") {
            false
        } else {
            return Err(format!("{} direction must be left or right.", id));
        };
        let xor = mask_from_hex_str(
            item.xor.as_ref().ok_or_else(|| format!("{} needs an xor mask.", id))?,
            bits,
        )?;
        let add = if let Some(v) = &item.add {
            mask_from_hex_str(&v.to_string(), bits)?
        } else {
            0
        };
        list.push(GmKeyRow {
            id,
            bits,
            rotate,
            left,
            xor,
            add,
            big_endian: big_endian_of(&item.endian)?,
        });
    }
    Ok(list)
}

fn read_checksums(root: &RootDoc) -> Result<(Vec<ChecksumEntry>, Vec<ChecksumDeclaration>), String> {
    let mut list = Vec::new();
    let mut declarations = Vec::new();
    let rows = root.checksums.as_ref();
    if rows.is_none() {
        return Ok((list, declarations));
    }
    for (index, item) in rows.unwrap().iter().enumerate() {
        let configured = flag(&item.configured)?;
        let id = optional_str(&item.id);
        let row_label = if id.is_empty() {
            format!("checksums[{}]", index)
        } else {
            id.clone()
        };
        let algorithm = optional_str(&item.algorithm).to_lowercase();
        if configured {
            if algorithm != "sum16" && algorithm != "sum32" && algorithm != "crc32" {
                return Err(format!(
                    "{}: algorithm must be sum16, sum32, or crc32.",
                    row_label
                ));
            }
            let start = integer_from_opt(item.start.as_ref()).map_err(|e| {
                format!("{} start: {}", row_label, e)
            })?;
            let end = integer_from_opt(item.end.as_ref()).map_err(|e| {
                format!("{} end: {}", row_label, e)
            })?;
            let store = integer_from_opt(item.store.as_ref()).map_err(|e| {
                format!("{} store: {}", row_label, e)
            })?;
            list.push(ChecksumEntry {
                profile: canonical_profile(&optional_str(&item.profile)),
                algorithm: algorithm.clone(),
                start,
                end,
                store,
                big_endian: big_endian_of(&item.endian)?,
            });
            declarations.push(ChecksumDeclaration {
                id: row_label,
                profile: optional_str(&item.profile),
                algorithm,
                configured_in_json: true,
                active: true,
            });
        } else {
            declarations.push(ChecksumDeclaration {
                id: row_label,
                profile: optional_str(&item.profile),
                algorithm: if algorithm.is_empty() {
                    "—".to_string()
                } else {
                    algorithm
                },
                configured_in_json: false,
                active: false,
            });
        }
    }
    Ok((list, declarations))
}

fn read_profiles(
    root: &RootDoc,
    generations: &[DensoGen],
    gm_keys: &[GmKeyRow],
    state: &mut PlatformState,
) -> Result<(), String> {
    let profiles = root.profiles.as_ref();
    if profiles.is_none() {
        return Ok(());
    }
    for (key, item) in profiles.unwrap() {
        if !flag(&item.configured)? {
            continue;
        }
        let id = canonical_profile(key);
        if id.is_empty() {
            return Err("A profile id is required.".to_string());
        }
        let algorithm = optional_str(&item.algorithm).to_lowercase();
        let kind = if algorithm == "denso32" {
            make_denso_profile(item, generations)?
        } else if algorithm == "gm" {
            make_gm_profile(item, gm_keys)?
        } else {
            return Err(format!("{} needs algorithm denso32 or gm.", id));
        };
        if let Some(kind) = kind {
            state.profile_keys.insert(id.clone(), kind);
            state.file_configured_profiles.insert(id);
        } else if state.load_note.is_empty() {
            state.load_note = format!("No security key function is configured for {}.", id);
        }
    }
    Ok(())
}

fn make_denso_profile(item: &ProfileDoc, generations: &[DensoGen]) -> Result<Option<ProfileKeyKind>, String> {
    let generation_id = optional_str(&item.generation);
    let (mask, big_endian) = if !generation_id.is_empty() {
        let gen = generations
            .iter()
            .find(|g| g.id.eq_ignore_ascii_case(&generation_id));
        if gen.is_none() {
            return Ok(None);
        }
        let gen = gen.unwrap();
        (gen.mask, gen.big_endian)
    } else {
        if item.mask.is_none() {
            return Ok(None);
        }
        let mask = mask_from_hex_str(item.mask.as_ref().ok_or("Denso profile needs a mask.")?, 32)?;
        (mask, big_endian_of(&item.endian)?)
    };
    Ok(Some(ProfileKeyKind::Denso { mask, big_endian }))
}

fn make_gm_profile(item: &ProfileDoc, keys: &[GmKeyRow]) -> Result<Option<ProfileKeyKind>, String> {
    let key_id = optional_str(&item.key_id);
    if key_id.is_empty() {
        return Ok(None);
    }
    let row = keys.iter().find(|k| k.id.eq_ignore_ascii_case(&key_id));
    if row.is_none() {
        return Ok(None);
    }
    let row = row.unwrap();
    Ok(Some(ProfileKeyKind::Gm {
        bits: row.bits,
        rotate: row.rotate,
        left: row.left,
        xor: row.xor,
        add: row.add,
        big_endian: row.big_endian,
    }))
}

fn dispatch_key(seed: &[u8], kind: &ProfileKeyKind) -> Result<Vec<u8>, String> {
    match kind {
        ProfileKeyKind::Denso { mask, big_endian } => {
            SecurityKeys::compute_denso32(seed, *mask, *big_endian)
        }
        ProfileKeyKind::Gm {
            bits,
            rotate,
            left,
            xor,
            add,
            big_endian,
        } => SecurityKeys::compute_gm(seed, *bits, *rotate, *left, *xor, *add, *big_endian),
    }
}

fn optional_str(v: &Option<String>) -> String {
    v.as_deref().unwrap_or("").trim().to_string()
}

fn flag(v: &Option<serde_json::Value>) -> Result<bool, String> {
    match v {
        None => Ok(false),
        Some(serde_json::Value::Bool(b)) => Ok(*b),
        Some(other) => {
            let text = other.to_string().trim_matches('"').to_string();
            if text.eq_ignore_ascii_case("true") {
                Ok(true)
            } else if text.eq_ignore_ascii_case("false") {
                Ok(false)
            } else {
                Err("configured must be true or false.".to_string())
            }
        }
    }
}

fn big_endian_of(endian: &Option<String>) -> Result<bool, String> {
    let text = optional_str(endian);
    if text.is_empty() || text.eq_ignore_ascii_case("big") {
        Ok(true)
    } else if text.eq_ignore_ascii_case("little") {
        Ok(false)
    } else {
        Err("The endian value must be big or little.".to_string())
    }
}

fn integer_from_opt(v: Option<&serde_json::Value>) -> Result<i64, String> {
    match v {
        None => Err("A number is missing.".to_string()),
        Some(val) => integer_val(&Some(val.clone())),
    }
}

fn integer_val(v: &Option<serde_json::Value>) -> Result<i64, String> {
    match v {
        None => Err("A number is missing.".to_string()),
        Some(serde_json::Value::Number(n)) => n
            .as_i64()
            .ok_or_else(|| "Invalid number.".to_string()),
        Some(serde_json::Value::String(s)) => parse_integer(s),
        Some(other) => parse_integer(&other.to_string()),
    }
}

fn parse_integer(text: &str) -> Result<i64, String> {
    let t = text.trim();
    if t.len() >= 2 && t[..2].eq_ignore_ascii_case("0x") {
        if t.len() < 3 {
            return Err("A hex number is empty.".to_string());
        }
        i64::from_str_radix(&t[2..], 16).map_err(|e| e.to_string())
    } else {
        t.parse::<i64>().map_err(|e| e.to_string())
    }
}

fn mask_from_hex_str(s: &str, bits: usize) -> Result<u32, String> {
    let parsed = parse_integer(s)?;
    if bits >= 32 {
        return Ok(parsed as u32);
    }
    let mask = (1u32 << bits) - 1;
    Ok((parsed as u32) & mask)
}

pub fn key_registration_label(reg: KeyRegistration) -> &'static str {
    match reg {
        KeyRegistration::None => "not loaded",
        KeyRegistration::Json => "loaded (JSON)",
        KeyRegistration::Bench => "bench (Virtual ECU)",
    }
}

pub fn security_mode_label(mode: SecurityMode) -> &'static str {
    match mode {
        SecurityMode::FailClosed => "Fail-closed",
        SecurityMode::Bench => "Bench",
        SecurityMode::Production => "Production",
    }
}

#[cfg(test)]
mod security_config_tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;

    fn write_temp_config(name: &str, body: &str) -> PathBuf {
        let path = std::env::temp_dir().join(name);
        let mut f = std::fs::File::create(&path).expect("temp config");
        f.write_all(body.as_bytes()).expect("write config");
        path
    }

    #[test]
    fn rejects_missing_profiles_object() {
        let _guard = PlatformSecurity::test_serial_guard();
        let path = write_temp_config(
            "wideband-sec-no-profiles.json",
            r#"{ "checksums": [] }"#,
        );
        PlatformSecurity::install(&path);
        let note = PlatformSecurity::load_note();
        assert!(note.contains("profiles"), "{}", note);
    }

    #[test]
    fn production_flash_blocked_with_bench_keys_only() {
        let _guard = PlatformSecurity::test_serial_guard();
        PlatformSecurity::install(&PathBuf::from("tests/security-config-test.json"));
        PlatformSecurity::register_bench_keys_for_virtual();
        let blockers = PlatformSecurity::flash_blockers("tacoma-can", false);
        assert!(
            blockers.iter().any(|b| b.contains("Production flash blocked")),
            "{:?}",
            blockers
        );
        let virtual_blockers = PlatformSecurity::flash_blockers("tacoma-can", true);
        assert!(
            virtual_blockers.is_empty(),
            "virtual bench flash should be allowed: {:?}",
            virtual_blockers
        );
    }

    #[test]
    fn fail_closed_when_profile_not_configured() {
        let _guard = PlatformSecurity::test_serial_guard();
        PlatformSecurity::install(&PathBuf::from("security-config.json"));
        let blockers = PlatformSecurity::flash_blockers("tacoma-can", false);
        assert!(!blockers.is_empty());
        assert!(blockers.iter().any(|b| b.contains("checksum") || b.contains("key")));
    }
}
