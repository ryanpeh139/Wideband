use crate::pack_paths::discover_definition_pack_roots;
use crate::plugin_host::PluginRegistry;
use crate::protocols::PluginRefs;
use serde::Deserialize;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

pub const SUPPORTED_DEFINITION_PACK_VERSION: u32 = 1;

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DefinitionFileRole {
    Xdf,
    A2l,
    Ini,
    Custom,
}

#[derive(Clone, Debug)]
pub struct DefinitionFileEntry {
    pub id: String,
    pub role: DefinitionFileRole,
    pub path: PathBuf,
}

#[derive(Clone, Debug)]
pub struct DefinitionPack {
    pub id: String,
    pub title: String,
    pub profile_ids: Vec<String>,
    pub primary_file_id: String,
    pub files: Vec<DefinitionFileEntry>,
    pub pack_dir: PathBuf,
    pub plugins: PluginRefs,
}

#[derive(Clone, Debug)]
pub struct DefinitionPackCatalog {
    pub packs: Vec<DefinitionPack>,
    pub load_note: String,
}

impl DefinitionPackCatalog {
    pub fn load_default() -> Self {
        let roots = discover_definition_pack_roots();
        Self::load_from_roots(&roots)
    }

    pub fn load_from_roots(roots: &[PathBuf]) -> Self {
        let mut packs = Vec::new();
        let mut errors = Vec::new();
        let mut ids = BTreeSet::new();

        for root in roots {
            if !root.is_dir() {
                continue;
            }
            let entries = match fs::read_dir(root) {
                Ok(e) => e,
                Err(e) => {
                    errors.push(format!("{}: {}", root.display(), e));
                    continue;
                }
            };
            for entry in entries.filter_map(|e| e.ok()) {
                let dir = entry.path();
                if !dir.is_dir() {
                    continue;
                }
                let manifest = dir.join("pack.json");
                if !manifest.is_file() {
                    continue;
                }
                match load_pack_dir(&dir, &manifest) {
                    Ok(pack) => {
                        if ids.contains(&pack.id) {
                            errors.push(format!(
                                "{}: duplicate definition pack id '{}'",
                                dir.display(),
                                pack.id
                            ));
                            continue;
                        }
                        if let Err(e) = PluginRegistry::require_refs(&pack.plugins) {
                            errors.push(format!("{}: {}", pack.id, e));
                            continue;
                        }
                        ids.insert(pack.id.clone());
                        packs.push(pack);
                    }
                    Err(e) => errors.push(format!("{}: {}", dir.display(), e)),
                }
            }
        }

        packs.sort_by(|a, b| a.title.cmp(&b.title));

        let load_note = if packs.is_empty() {
            if errors.is_empty() {
                "No community definition packs discovered.".to_string()
            } else {
                format!("Definition pack load failed: {}", errors.join(" | "))
            }
        } else if errors.is_empty() {
            format!("Loaded {} definition pack(s).", packs.len())
        } else {
            format!(
                "Loaded {} definition pack(s) with warnings: {}",
                packs.len(),
                errors.join(" | ")
            )
        };

        Self { packs, load_note }
    }

    pub fn best_for_profile(&self, profile_id: &str) -> Option<&DefinitionPack> {
        self.packs
            .iter()
            .find(|p| p.profile_ids.iter().any(|id| id == profile_id))
    }

    pub fn primary_xdf_path(&self, profile_id: &str) -> Option<PathBuf> {
        let pack = self.best_for_profile(profile_id)?;
        pack.files
            .iter()
            .find(|f| f.id == pack.primary_file_id && f.role == DefinitionFileRole::Xdf)
            .map(|f| f.path.clone())
            .or_else(|| {
                pack.files
                    .iter()
                    .find(|f| f.role == DefinitionFileRole::Xdf)
                    .map(|f| f.path.clone())
            })
    }
}

fn load_pack_dir(dir: &Path, manifest_path: &Path) -> Result<DefinitionPack, String> {
    let text = fs::read_to_string(manifest_path).map_err(|e| e.to_string())?;
    let root: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("invalid JSON: {}", e))?;
    reject_embedded_secrets(&root)?;
    let file: DefinitionPackFile =
        serde_json::from_value(root).map_err(|e| format!("schema mismatch: {}", e))?;
    file.into_pack(dir)
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
            Ok(())
        }
        _ => Err("definition pack root must be a JSON object".to_string()),
    }
}

#[derive(Debug, Deserialize)]
struct DefinitionPackFile {
    pack_version: u32,
    id: String,
    title: String,
    #[serde(default)]
    profile_ids: Vec<String>,
    primary: String,
    files: Vec<DefinitionFileSpec>,
    #[serde(default)]
    plugins: PluginRefsFile,
}

#[derive(Debug, Deserialize, Default)]
struct PluginRefsFile {
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
struct DefinitionFileSpec {
    id: String,
    role: String,
    path: String,
    #[serde(default)]
    optional: bool,
}

impl DefinitionPackFile {
    fn into_pack(self, dir: &Path) -> Result<DefinitionPack, String> {
        if self.pack_version != SUPPORTED_DEFINITION_PACK_VERSION {
            return Err(format!(
                "unsupported pack_version {} (expected {})",
                self.pack_version,
                SUPPORTED_DEFINITION_PACK_VERSION
            ));
        }
        if self.id.trim().is_empty() {
            return Err("id must not be empty".to_string());
        }
        if self.files.is_empty() {
            return Err("files must not be empty".to_string());
        }
        let primary = self.primary.trim();
        if primary.is_empty() {
            return Err("primary must name a file id".to_string());
        }

        let mut files = Vec::new();
        let mut seen_ids = BTreeSet::new();
        for spec in self.files {
            if spec.id.trim().is_empty() {
                return Err("file id must not be empty".to_string());
            }
            if seen_ids.contains(&spec.id) {
                return Err(format!("duplicate file id '{}'", spec.id));
            }
            seen_ids.insert(spec.id.clone());
            let role = parse_role(&spec.role)?;
            let path = dir.join(&spec.path);
            if !path.is_file() {
                if spec.optional {
                    continue;
                }
                return Err(format!("missing file {}", path.display()));
            }
            files.push(DefinitionFileEntry {
                id: spec.id,
                role,
                path,
            });
        }
        if files.is_empty() {
            return Err("no usable files after optional filtering".to_string());
        }
        if !files.iter().any(|f| f.id == primary) {
            return Err(format!("primary file id '{}' not found in files", primary));
        }

        Ok(DefinitionPack {
            id: self.id.trim().to_string(),
            title: self.title,
            profile_ids: self.profile_ids,
            primary_file_id: primary.to_string(),
            files,
            pack_dir: dir.to_path_buf(),
            plugins: PluginRefs {
                checksum: self.plugins.checksum,
                protocol: self.plugins.protocol,
                patch: self.plugins.patch,
                manager: self.plugins.manager,
            },
        })
    }
}

fn parse_role(raw: &str) -> Result<DefinitionFileRole, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "xdf" => Ok(DefinitionFileRole::Xdf),
        "a2l" | "damos" => Ok(DefinitionFileRole::A2l),
        "ini" | "logger" => Ok(DefinitionFileRole::Ini),
        "custom" => Ok(DefinitionFileRole::Custom),
        other => Err(format!("unknown file role '{}'", other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_shipped_tacoma_bench_definition_pack() {
        let catalog = DefinitionPackCatalog::load_from_roots(&[PathBuf::from("definition-packs")]);
        let pack = catalog
            .packs
            .iter()
            .find(|p| p.id == "tacoma-can-bench")
            .expect(&catalog.load_note);
        assert!(pack.files.iter().any(|f| f.role == DefinitionFileRole::Xdf));
    }

    #[test]
    fn rejects_embedded_key_material() {
        let json = r#"{"pack_version":1,"id":"x","title":"X","primary":"a","mask":"1","files":[]}"#;
        let root: serde_json::Value = serde_json::from_str(json).unwrap();
        assert!(reject_embedded_secrets(&root).is_err());
    }
}
