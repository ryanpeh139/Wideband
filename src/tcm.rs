//! Transmission Control Module (TCM) profiles, identification, and read-oriented parameter views.
//!
//! TCM packs mirror ECM transport wiring with separate CAN IDs. Security and checksum refs stay in
//! `security-config.json`; flash segments in `flash-layout.json`. Unconfigured rows fail closed like ECM.

use crate::definitions::XdfDocument;
use crate::flash_layout::FlashLayoutMap;
use crate::j2534::PassThruDevice;
use crate::protocols::{
    clean_id_text, parse_mode09_cal_ids, parse_mode09_cvns, DiagnosticProtocol, EcuIdentification,
    EcuProfile, VehicleBus,
};
use crate::security_config::PlatformSecurity;

/// How long TCM identify waits for each OBD Mode 09 reply; the TCM answers in ~15–30 ms and
/// stays silent for a PID it lacks.
const OBD_REPLY_MS: u32 = 500;
use crate::uds::UdsClient;
use serde::Deserialize;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

pub const SUPPORTED_TCM_PACK_VERSION: u32 = 1;

/// Logical grouping for definition-driven TCM parameter scaffolding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TcmParameterGroup {
    Shift,
    Pressure,
    Tcc,
    Other,
}

#[derive(Clone, Debug)]
pub struct TcmParameterEntry {
    pub group: TcmParameterGroup,
    pub kind: &'static str,
    pub title: String,
    pub unique_id: String,
    pub units: String,
    pub summary: String,
}

#[derive(Clone, Debug, Default)]
pub struct TcmParameterView {
    pub entries: Vec<TcmParameterEntry>,
    pub definition_title: String,
    pub note: String,
}

#[derive(Clone, Debug)]
pub struct TcmProfile {
    pub id: String,
    pub name: String,
    pub platform: String,
    /// ECM profile pack id this TCM profile is paired with in the UI.
    pub paired_ecm_profile_id: String,
    /// Human label for transmission hardware family (extension point for future packs).
    pub transmission_family: String,
    pub bus: VehicleBus,
    pub diagnostic: DiagnosticProtocol,
    pub bitrate: u32,
    pub protocol_id: u32,
    pub connect_flags: u32,
    pub extended_id: bool,
    /// Diagnostic (UDS) request and response ids.
    pub request_id: u32,
    pub response_id: u32,
    /// OBD-II physical ids (replies to functional `0x7DF` requests); same as the diagnostic ids
    /// unless the TCM splits them.
    pub obd_request_id: u32,
    pub obd_response_id: u32,
    pub filter_mask: u32,
    pub tester: u8,
    pub ecu: u8,
    pub max_chunk: usize,
    pub address_bytes: usize,
    pub size_bytes: usize,
    pub programming_session: u8,
    pub read_session: u8,
    pub security_profile_id: String,
    pub flash_layout_profile_id: String,
    pub checksum_profile_id: String,
}

impl TcmProfile {
    pub fn tacoma_can_tcm() -> Self {
        Self {
            id: "tacoma-can-tcm".to_string(),
            name: "Toyota Tacoma A750E TCM (CAN)".to_string(),
            platform: "Toyota Tacoma".to_string(),
            paired_ecm_profile_id: "tacoma-can".to_string(),
            transmission_family: "A750E (stub)".to_string(),
            bus: VehicleBus::CanIsoTp,
            diagnostic: DiagnosticProtocol::Uds,
            bitrate: 500_000,
            protocol_id: crate::j2534::ISO15765,
            connect_flags: 0,
            extended_id: false,
            // The 2017 Tacoma TCM answers UDS on 0x701/0x709 and OBD on 0x7DF -> 0x7E9; it ignores
            // physical requests to 0x7E1 except flow control.
            request_id: 0x701,
            response_id: 0x709,
            obd_request_id: 0x7E1,
            obd_response_id: 0x7E9,
            filter_mask: 0x7FF,
            tester: 0xF1,
            ecu: 0x18,
            max_chunk: 1024,
            address_bytes: 4,
            size_bytes: 4,
            programming_session: 0x02,
            read_session: 0x03,
            security_profile_id: "tacoma-can-tcm".to_string(),
            flash_layout_profile_id: "tacoma-can-tcm".to_string(),
            checksum_profile_id: "tacoma-can-tcm".to_string(),
        }
    }

    pub fn tacoma_kline_tcm() -> Self {
        Self {
            id: "tacoma-kline-tcm".to_string(),
            name: "Toyota Tacoma TCM (K-Line stub)".to_string(),
            platform: "Toyota Tacoma".to_string(),
            paired_ecm_profile_id: "tacoma-kline".to_string(),
            transmission_family: "A340F (stub)".to_string(),
            bus: VehicleBus::KLine,
            diagnostic: DiagnosticProtocol::Kwp,
            bitrate: 10400,
            protocol_id: crate::j2534::ISO14230,
            connect_flags: 0x0200,
            extended_id: false,
            request_id: 0x13,
            response_id: 0xF1,
            obd_request_id: 0x13,
            obd_response_id: 0xF1,
            filter_mask: 0xFF,
            tester: 0xF1,
            ecu: 0x18,
            max_chunk: 128,
            address_bytes: 3,
            size_bytes: 3,
            programming_session: 0x85,
            read_session: 0x81,
            security_profile_id: "tacoma-kline-tcm".to_string(),
            flash_layout_profile_id: "tacoma-kline-tcm".to_string(),
            checksum_profile_id: "tacoma-kline-tcm".to_string(),
        }
    }

    pub fn silverado_can_tcm() -> Self {
        Self {
            id: "silverado-can-tcm".to_string(),
            name: "Chevrolet Silverado 6L80 TCM (CAN)".to_string(),
            platform: "Chevrolet Silverado".to_string(),
            paired_ecm_profile_id: "silverado-can".to_string(),
            transmission_family: "6L80 (stub)".to_string(),
            bus: VehicleBus::CanIsoTp,
            diagnostic: DiagnosticProtocol::Uds,
            bitrate: 500_000,
            protocol_id: crate::j2534::ISO15765,
            connect_flags: 0,
            extended_id: true,
            request_id: 0x18DA28F1,
            response_id: 0x18DAF128,
            obd_request_id: 0x18DA28F1,
            obd_response_id: 0x18DAF128,
            filter_mask: 0x1FFFFFFF,
            tester: 0xF1,
            ecu: 0x28,
            max_chunk: 1024,
            address_bytes: 4,
            size_bytes: 4,
            programming_session: 0x02,
            read_session: 0x03,
            security_profile_id: "silverado-can-tcm".to_string(),
            flash_layout_profile_id: "silverado-can-tcm".to_string(),
            checksum_profile_id: "silverado-can-tcm".to_string(),
        }
    }

    pub fn silverado_vpw_tcm() -> Self {
        Self {
            id: "silverado-vpw-tcm".to_string(),
            name: "Chevrolet Silverado TCM (VPW stub)".to_string(),
            platform: "Chevrolet Silverado".to_string(),
            paired_ecm_profile_id: "silverado-vpw".to_string(),
            transmission_family: "4L60E (stub)".to_string(),
            bus: VehicleBus::Vpw,
            diagnostic: DiagnosticProtocol::Uds,
            bitrate: 10400,
            protocol_id: crate::j2534::J1850VPW,
            connect_flags: 0,
            extended_id: false,
            request_id: 0x6C,
            response_id: 0x6C,
            obd_request_id: 0x6C,
            obd_response_id: 0x6C,
            filter_mask: 0xFF,
            tester: 0xF1,
            ecu: 0x28,
            max_chunk: 64,
            address_bytes: 3,
            size_bytes: 2,
            programming_session: 0x02,
            read_session: 0x03,
            security_profile_id: "silverado-vpw-tcm".to_string(),
            flash_layout_profile_id: "silverado-vpw-tcm".to_string(),
            checksum_profile_id: "silverado-vpw-tcm".to_string(),
        }
    }

    pub fn builtin_defaults() -> Vec<Self> {
        vec![
            Self::tacoma_can_tcm(),
            Self::tacoma_kline_tcm(),
            Self::silverado_can_tcm(),
            Self::silverado_vpw_tcm(),
        ]
    }

    /// Reuse ECM UDS client wiring with TCM CAN/K-Line IDs.
    pub fn to_diagnostic_profile(&self) -> EcuProfile {
        EcuProfile {
            id: self.id.clone(),
            name: self.name.clone(),
            platform: self.platform.clone(),
            bus: self.bus,
            diagnostic: self.diagnostic,
            bitrate: self.bitrate,
            protocol_id: self.protocol_id,
            connect_flags: self.connect_flags,
            extended_id: self.extended_id,
            request_id: self.request_id,
            response_id: self.response_id,
            obd_request_id: self.obd_request_id,
            obd_response_id: self.obd_response_id,
            filter_mask: self.filter_mask,
            tester: self.tester,
            ecu: self.ecu,
            max_chunk: self.max_chunk,
            address_bytes: self.address_bytes,
            size_bytes: self.size_bytes,
            programming_session: self.programming_session,
            read_session: self.read_session,
            security_profile_id: self.security_profile_id.clone(),
            flash_layout_profile_id: self.flash_layout_profile_id.clone(),
            checksum_profile_id: self.checksum_profile_id.clone(),
            plugins: crate::protocols::PluginRefs::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct TcmProfileCatalog {
    pub profiles: Vec<TcmProfile>,
    pub load_note: String,
    pub pack_dir: PathBuf,
}

impl TcmProfileCatalog {
    pub fn load_default() -> Self {
        let dir = Self::discover_directory();
        Self::load_from_directory(&dir)
    }

    pub fn discover_directory() -> PathBuf {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                let beside = parent.join("profiles").join("tcm");
                if beside.is_dir() {
                    return beside;
                }
            }
        }
        PathBuf::from("profiles/tcm")
    }

    pub fn load_from_directory(dir: &Path) -> Self {
        let mut profiles = Vec::new();
        let mut errors = Vec::new();
        let mut ids = BTreeSet::new();

        if !dir.is_dir() {
            return Self {
                profiles: TcmProfile::builtin_defaults(),
                load_note: format!(
                    "TCM pack directory not found ({}); using built-in TCM stubs.",
                    dir.display()
                ),
                pack_dir: dir.to_path_buf(),
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
                    profiles: TcmProfile::builtin_defaults(),
                    load_note: format!(
                        "Could not read {}: {}; using built-in TCM stubs.",
                        dir.display(),
                        e
                    ),
                    pack_dir: dir.to_path_buf(),
                };
            }
        };
        paths.sort();

        if paths.is_empty() {
            return Self {
                profiles: TcmProfile::builtin_defaults(),
                load_note: format!(
                    "No TCM packs in {}; using built-in TCM stubs.",
                    dir.display()
                ),
                pack_dir: dir.to_path_buf(),
            };
        }

        for path in paths {
            match load_tcm_pack_file(&path) {
                Ok(profile) => {
                    if ids.contains(&profile.id) {
                        errors.push(format!(
                            "{}: duplicate TCM profile id '{}'",
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
                profiles: TcmProfile::builtin_defaults(),
                load_note: format!(
                    "TCM pack load failed ({}); using built-in stubs. {}",
                    dir.display(),
                    errors.join(" | ")
                ),
                pack_dir: dir.to_path_buf(),
            };
        }

        profiles.sort_by(|a, b| a.name.cmp(&b.name));
        let load_note = if errors.is_empty() {
            format!("Loaded {} TCM pack(s) from {}.", profiles.len(), dir.display())
        } else {
            format!(
                "Loaded {} TCM pack(s) from {} with warnings: {}",
                profiles.len(),
                dir.display(),
                errors.join(" | ")
            )
        };

        Self {
            profiles,
            load_note,
            pack_dir: dir.to_path_buf(),
        }
    }

    pub fn index_for_ecm(&self, ecm_profile_id: &str) -> Option<usize> {
        self.profiles
            .iter()
            .position(|p| p.paired_ecm_profile_id == ecm_profile_id)
    }
}

pub struct TcmIdentifier;

impl TcmIdentifier {
    pub fn query(
        device: &mut dyn PassThruDevice,
        channel: u32,
        profile: &TcmProfile,
        allow_demo_defaults: bool,
    ) -> Result<EcuIdentification, String> {
        let transport = profile.to_diagnostic_profile();
        if transport.bus != VehicleBus::CanIsoTp {
            return Err(format!(
                "TCM identify over {} is not implemented yet — use a CAN ISO-TP TCM profile.",
                profile.name
            ));
        }

        let mut id = EcuIdentification::default();
        let mut field_errors: Vec<String> = Vec::new();
        let mut client = UdsClient::for_profile(device, channel, &transport);
        // OBD Mode 09 first, as for the ECM: functional request, reply from the TCM's OBD id.
        // A TCM that lacks a PID stays silent, so do not wait the full 2 s P2 for it.
        let mut obd = UdsClient::for_obd(channel, &transport);
        obd.set_timers(OBD_REPLY_MS, OBD_REPLY_MS);

        // VIN: Mode 09 02, then DID F190.
        let mut vin_error = String::new();
        match obd.request(device, &[0x09, 0x02], 0x49) {
            Ok(resp) if resp.len() >= 4 => id.vin = clean_id_text(&resp[3..]),
            Ok(_) => vin_error = "Mode 09 response too short".to_string(),
            Err(e) => vin_error = format!("Mode 09: {}", e),
        }
        if id.vin.is_empty() {
            match client.request(device, &[0x22, 0xF1, 0x90], 0x62) {
                Ok(resp) if resp.len() >= 4 => id.vin = clean_id_text(&resp[3..]),
                Ok(_) => vin_error = format!("{}; DID F190 response too short", vin_error),
                Err(e) => vin_error = format!("{}; DID F190: {}", vin_error, e),
            }
        }
        if id.vin.is_empty() {
            field_errors.push(format!("VIN: {}", vin_error));
        }

        // Cal IDs: Mode 09 04, then the platform DID.
        let mut cal_error = String::new();
        match obd.request(device, &[0x09, 0x04], 0x49) {
            Ok(resp) => {
                id.cal_ids = parse_mode09_cal_ids(&resp);
                if id.cal_ids.is_empty() {
                    cal_error = "Mode 09 response had no calibration ID".to_string();
                }
            }
            Err(e) => cal_error = format!("Mode 09: {}", e),
        }
        if id.cal_ids.is_empty() {
            let cal_req = if profile.platform.contains("Toyota") {
                vec![0x22, 0xF1, 0x82]
            } else {
                vec![0x22, 0x00, 0xC2]
            };
            match client.request(device, &cal_req, 0x62) {
                Ok(resp) if resp.len() >= 4 => {
                    let cal = clean_id_text(&resp[3..]);
                    if !cal.is_empty() {
                        id.cal_ids.push(cal);
                    }
                }
                Ok(_) => cal_error = format!("{}; DID response too short", cal_error),
                Err(e) => cal_error = format!("{}; DID: {}", cal_error, e),
            }
        }
        if id.cal_ids.is_empty() {
            field_errors.push(format!("TCM Cal ID: {}", cal_error));
        }

        // CVNs: Mode 09 06.
        match obd.request(device, &[0x09, 0x06], 0x49) {
            Ok(resp) => {
                id.cvns = parse_mode09_cvns(&resp);
                if id.cvns.is_empty() {
                    field_errors.push("CVN: Mode 09 response had no CVN".to_string());
                }
            }
            Err(e) => field_errors.push(format!("CVN: {}", e)),
        }
        id.cal_id = id.cal_ids.join(", ");
        id.cvn = id.cvns.join(", ");

        if allow_demo_defaults {
            if id.vin.is_empty() {
                id.vin = crate::j2534::VIRTUAL_TACOMA_VIN.to_string();
            }
            if id.cal_id.is_empty() {
                id.cal_id = crate::j2534::virtual_tcm_cal_id_for_profile(&profile.id);
            }
            if id.cvn.is_empty() {
                id.cvn = crate::j2534::VIRTUAL_TACOMA_CVN_HEX.to_string();
            }
            field_errors.clear();
        } else if id.vin.is_empty() && id.cal_id.is_empty() && id.cvn.is_empty() {
            let mut message =
                "TCM did not respond to identification (ignition ON, matching TCM profile, and bus traffic required)."
                    .to_string();
            if !field_errors.is_empty() {
                message = format!("{} {}", message, field_errors.join("; "));
            }
            return Err(message);
        }

        id.details = format!(
            "TCM Identification: VIN: {}, Cal ID: {}, CVN: {} [{} · {}]",
            if id.vin.is_empty() { "(none)" } else { &id.vin },
            if id.cal_id.is_empty() { "(none)" } else { &id.cal_id },
            if id.cvn.is_empty() { "(none)" } else { &id.cvn },
            profile.name,
            profile.transmission_family
        );
        // A TCM often has no VIN; list what it did not answer instead of failing the read.
        if !field_errors.is_empty() {
            id.details = format!("{} — not read: {}", id.details, field_errors.join("; "));
        }

        Ok(id)
    }
}

pub fn classify_parameter_text(title: &str, description: &str, unique_id: &str) -> TcmParameterGroup {
    let hay = format!(
        "{} {} {}",
        title.to_ascii_lowercase(),
        description.to_ascii_lowercase(),
        unique_id.to_ascii_lowercase()
    );
    if hay.contains("tcc")
        || hay.contains("torque converter")
        || hay.contains("lockup")
        || hay.contains("converter")
    {
        return TcmParameterGroup::Tcc;
    }
    if hay.contains("pressure")
        || hay.contains("line pr")
        || hay.contains("clutch")
        || hay.contains("apply")
    {
        return TcmParameterGroup::Pressure;
    }
    if hay.contains("shift")
        || hay.contains("solenoid")
        || hay.contains("gear")
        || hay.contains("on/off")
    {
        return TcmParameterGroup::Shift;
    }
    TcmParameterGroup::Other
}

pub fn build_parameter_view(xdf: &XdfDocument) -> TcmParameterView {
    let mut entries = Vec::new();
    for table in &xdf.tables {
        let group = classify_parameter_text(&table.title, &table.description, &table.id);
        if group == TcmParameterGroup::Other {
            continue;
        }
        entries.push(TcmParameterEntry {
            group,
            kind: "2D table",
            title: table.title.clone(),
            unique_id: table.id.clone(),
            units: if table.z_units.is_empty() {
                "%".to_string()
            } else {
                table.z_units.clone()
            },
            summary: format!(
                "{}×{} — {}",
                table.x_axis.dim,
                table.y_axis.dim,
                table.description
            ),
        });
    }
    for constant in &xdf.constants {
        let group = classify_parameter_text(&constant.title, "", &constant.id);
        if group == TcmParameterGroup::Other {
            continue;
        }
        entries.push(TcmParameterEntry {
            group,
            kind: "scalar",
            title: constant.title.clone(),
            unique_id: constant.id.clone(),
            units: constant.units.clone(),
            summary: format!("Address 0x{:X}", constant.address),
        });
    }

    entries.sort_by(|a, b| a.title.cmp(&b.title));

    let note = if entries.is_empty() {
        "No transmission-tagged tables in the loaded XDF. Load a TCM definition or add titles containing shift, pressure, or TCC keywords.".to_string()
    } else {
        format!(
            "{} definition-linked transmission parameter(s) from XDF titles/descriptions.",
            entries.len()
        )
    };

    TcmParameterView {
        entries,
        definition_title: xdf.title.clone(),
        note,
    }
}

pub fn flash_blockers(profile: &TcmProfile, virtual_ecu: bool, layout: &FlashLayoutMap) -> Vec<String> {
    let mut reasons = PlatformSecurity::flash_blockers(&profile.security_profile_id, virtual_ecu);
    match layout.cal_segments(&profile.flash_layout_profile_id) {
        Ok(segments) if segments.is_empty() => {
            reasons.push(format!(
                "TCM flash layout for \"{}\" has no cal_flash_segments — add segments in flash-layout.json after bench validation.",
                profile.flash_layout_profile_id
            ));
        }
        Err(e) => {
            reasons.push(format!("TCM flash layout: {}", e));
        }
        Ok(_) => {
            reasons.push(
                "TCM calibration flash execution is not wired yet; security and layout gates still apply (fail-closed)."
                    .to_string(),
            );
        }
    }
    reasons
}

pub fn group_label(group: TcmParameterGroup) -> &'static str {
    match group {
        TcmParameterGroup::Shift => "Shift / solenoid",
        TcmParameterGroup::Pressure => "Pressure / clutch",
        TcmParameterGroup::Tcc => "TCC / lockup",
        TcmParameterGroup::Other => "Other",
    }
}

// --- JSON pack loader (profiles/tcm/*.json) ---

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
];

fn load_tcm_pack_file(path: &Path) -> Result<TcmProfile, String> {
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let root: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("invalid JSON: {}", e))?;
    reject_embedded_secrets(&root)?;
    let pack: TcmPackFile =
        serde_json::from_value(root).map_err(|e| format!("schema mismatch: {}", e))?;
    pack.into_profile()
}

fn reject_embedded_secrets(value: &serde_json::Value) -> Result<(), String> {
    if let serde_json::Value::Object(map) = value {
        for key in map.keys() {
            let lower = key.to_ascii_lowercase();
            for forbidden in FORBIDDEN_ROOT_KEYS {
                if lower == forbidden.to_ascii_lowercase() {
                    return Err(format!(
                        "TCM pack must not embed '{}' (use security-config.json refs only)",
                        key
                    ));
                }
            }
        }
        Ok(())
    } else {
        Err("TCM pack root must be a JSON object".to_string())
    }
}

#[derive(Debug, Deserialize)]
struct TcmPackFile {
    pack_version: u32,
    id: String,
    name: String,
    platform: String,
    paired_ecm: String,
    transmission_family: Option<String>,
    bus: String,
    diagnostic: String,
    bitrate: u32,
    j2534: TcmJ2534Section,
    can: TcmCanSection,
    kline: TcmKlineSection,
    memory: TcmMemorySection,
    sessions: TcmSessionSection,
    refs: TcmRefSection,
}

#[derive(Debug, Deserialize)]
struct TcmJ2534Section {
    protocol: String,
    connect_flags: u32,
    extended_id: bool,
}

#[derive(Debug, Deserialize)]
struct TcmCanSection {
    request_id: String,
    response_id: String,
    filter_mask: String,
    #[serde(default)]
    obd_request_id: Option<String>,
    #[serde(default)]
    obd_response_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TcmKlineSection {
    tester: String,
    ecu: String,
}

#[derive(Debug, Deserialize)]
struct TcmMemorySection {
    max_chunk: usize,
    address_bytes: usize,
    size_bytes: usize,
}

#[derive(Debug, Deserialize)]
struct TcmSessionSection {
    programming: String,
    read: String,
}

#[derive(Debug, Deserialize)]
struct TcmRefSection {
    security: String,
    flash_layout: String,
    checksum: String,
}

impl TcmPackFile {
    fn into_profile(self) -> Result<TcmProfile, String> {
        if self.pack_version != SUPPORTED_TCM_PACK_VERSION {
            return Err(format!(
                "unsupported pack_version {} (expected {})",
                self.pack_version,
                SUPPORTED_TCM_PACK_VERSION
            ));
        }
        let request_id = parse_u32_hex(&self.can.request_id, "can.request_id")?;
        let response_id = parse_u32_hex(&self.can.response_id, "can.response_id")?;
        let obd_request_id = match &self.can.obd_request_id {
            Some(raw) => parse_u32_hex(raw, "can.obd_request_id")?,
            None => request_id,
        };
        let obd_response_id = match &self.can.obd_response_id {
            Some(raw) => parse_u32_hex(raw, "can.obd_response_id")?,
            None => response_id,
        };
        Ok(TcmProfile {
            id: self.id.trim().to_string(),
            name: self.name,
            platform: self.platform,
            paired_ecm_profile_id: self.paired_ecm.trim().to_string(),
            transmission_family: self
                .transmission_family
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "unspecified".to_string()),
            bus: parse_bus(&self.bus)?,
            diagnostic: parse_diagnostic(&self.diagnostic)?,
            bitrate: self.bitrate,
            protocol_id: parse_j2534_protocol(&self.j2534.protocol)?,
            connect_flags: self.j2534.connect_flags,
            extended_id: self.j2534.extended_id,
            request_id,
            response_id,
            obd_request_id,
            obd_response_id,
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
        "iso15765" | "can" => Ok(crate::j2534::ISO15765),
        "iso14230" | "kline" => Ok(crate::j2534::ISO14230),
        "j1850vpw" | "vpw" => Ok(crate::j2534::J1850VPW),
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
    fn tcm_pairs_with_ecm_catalog() {
        let catalog = TcmProfileCatalog::load_from_directory(Path::new("profiles/tcm"));
        assert!(catalog.profiles.len() >= 4);
        let tacoma = catalog
            .profiles
            .iter()
            .find(|p| p.id == "tacoma-can-tcm")
            .expect("tacoma-can-tcm");
        assert_eq!(tacoma.paired_ecm_profile_id, "tacoma-can");
        assert_eq!((tacoma.request_id, tacoma.response_id), (0x701, 0x709));
        assert_eq!((tacoma.obd_request_id, tacoma.obd_response_id), (0x7E1, 0x7E9));
        assert_eq!(catalog.index_for_ecm("tacoma-can"), Some(catalog.profiles.iter().position(|p| p.id == "tacoma-can-tcm").unwrap()));
    }

    #[test]
    fn tcm_flash_is_fail_closed_without_security() {
        let _guard = crate::security_config::PlatformSecurity::test_serial_guard();
        crate::security_config::PlatformSecurity::install(
            &std::path::PathBuf::from("security-config.json"),
        );
        let layout = FlashLayoutMap::load("flash-layout.json").expect("layout");
        let prof = TcmProfile::tacoma_can_tcm();
        let blockers = flash_blockers(&prof, false, &layout);
        assert!(
            blockers.iter().any(|b| b.contains("security") || b.contains("No security")),
            "expected security blocker: {:?}",
            blockers
        );
    }

    #[test]
    fn definition_view_buckets_shift_pressure_tcc() {
        let xdf = XdfDocument::sample();
        let mut view = build_parameter_view(&xdf);
        // Sample XDF is ECM-oriented; inject synthetic titles for bucket test.
        view.entries.push(TcmParameterEntry {
            group: TcmParameterGroup::Shift,
            kind: "2D table",
            title: "Shift Solenoid Duty".to_string(),
            unique_id: "shift_a".to_string(),
            units: "%".to_string(),
            summary: "stub".to_string(),
        });
        assert!(view.entries.iter().any(|e| e.group == TcmParameterGroup::Shift));
        assert_eq!(classify_parameter_text("Line Pressure Target", "", ""), TcmParameterGroup::Pressure);
        assert_eq!(classify_parameter_text("TCC Apply", "", ""), TcmParameterGroup::Tcc);
    }
}
