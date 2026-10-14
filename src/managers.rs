use crate::definitions::XdfDocument;
use crate::strategy_meta::{StrategyNode, build_strategy_graph};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Deserialize)]
pub struct ManagerRegistryFile {
    pub version: u32,
    pub managers: Vec<ManagerDefinition>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ManagerDefinition {
    pub id: String,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub match_rules: ManagerMatchRules,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ManagerMatchRules {
    #[serde(default)]
    pub manager_ids: Vec<String>,
    #[serde(default)]
    pub title_contains: Vec<String>,
    #[serde(default)]
    pub param_ids: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ManagerPanel {
    pub definition: ManagerDefinition,
    pub members: Vec<StrategyNode>,
}

pub struct ManagerRegistry {
    pub file: ManagerRegistryFile,
    pub path: PathBuf,
}

impl ManagerRegistry {
    pub fn default_path() -> PathBuf {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                let beside = parent.join("strategy-managers.json");
                if beside.is_file() {
                    return beside;
                }
            }
        }
        PathBuf::from("strategy-managers.json")
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let raw = fs::read_to_string(path).map_err(|e| format!("read {}: {}", path.display(), e))?;
        let file: ManagerRegistryFile =
            serde_json::from_str(&raw).map_err(|e| format!("parse strategy-managers.json: {}", e))?;
        Ok(Self {
            file,
            path: path.to_path_buf(),
        })
    }

    pub fn builtin_fallback() -> Self {
        let raw = include_str!("../strategy-managers.json");
        let file: ManagerRegistryFile =
            serde_json::from_str(raw).expect("embedded strategy-managers.json");
        Self {
            file,
            path: PathBuf::from("strategy-managers.json"),
        }
    }

    pub fn load_or_embedded() -> Self {
        let path = Self::default_path();
        Self::load(&path).unwrap_or_else(|e| {
            eprintln!("strategy-managers: {}; using embedded defaults", e);
            Self::builtin_fallback()
        })
    }

    pub fn panels_for_xdf(&self, xdf: &XdfDocument) -> Vec<ManagerPanel> {
        let graph = build_strategy_graph(xdf);
        self.file
            .managers
            .iter()
            .map(|def| {
                let members = graph
                    .nodes
                    .iter()
                    .filter(|n| node_matches_manager(n, def))
                    .cloned()
                    .collect();
                ManagerPanel {
                    definition: def.clone(),
                    members,
                }
            })
            .collect()
    }
}

fn node_matches_manager(node: &StrategyNode, def: &ManagerDefinition) -> bool {
    let rules = &def.match_rules;
    if rules.param_ids.iter().any(|id| id == &node.id) {
        return true;
    }
    if let Some(mgr) = &node.meta.manager {
        if rules.manager_ids.iter().any(|id| id == mgr) {
            return true;
        }
    }
    for tag in &node.meta.tags {
        if rules.tags.iter().any(|t| t == tag) {
            return true;
        }
    }
    let title_l = node.title.to_ascii_lowercase();
    for needle in &rules.title_contains {
        if title_l.contains(&needle.to_ascii_lowercase()) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definitions::XdfDocument;

    #[test]
    fn registry_loads_embedded() {
        let reg = ManagerRegistry::builtin_fallback();
        assert!(!reg.file.managers.is_empty());
        let panels = reg.panels_for_xdf(&XdfDocument::sample());
        assert!(panels.iter().any(|p| p.definition.id == "boost"));
    }
}
