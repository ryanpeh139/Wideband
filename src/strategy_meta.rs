use crate::definitions::XdfDocument;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Parsed from XDF `description` (or title) via `wb-meta:` tag — plain text, no encrypted defs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WidebandMeta {
    pub manager: Option<String>,
    pub role: StrategyRole,
    pub depends: Vec<String>,
    pub tags: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum StrategyRole {
    #[default]
    Table,
    Target,
    Actuator,
    Limiter,
    Sensor,
    Logic,
    Constant,
    Flag,
}

impl StrategyRole {
    pub fn label(self) -> &'static str {
        match self {
            StrategyRole::Table => "table",
            StrategyRole::Target => "target",
            StrategyRole::Actuator => "actuator",
            StrategyRole::Limiter => "limiter",
            StrategyRole::Sensor => "sensor",
            StrategyRole::Logic => "logic",
            StrategyRole::Constant => "constant",
            StrategyRole::Flag => "flag",
        }
    }

    fn from_token(s: &str) -> StrategyRole {
        match s.trim().to_ascii_lowercase().as_str() {
            "target" => StrategyRole::Target,
            "actuator" => StrategyRole::Actuator,
            "limiter" => StrategyRole::Limiter,
            "sensor" => StrategyRole::Sensor,
            "logic" => StrategyRole::Logic,
            "constant" => StrategyRole::Constant,
            "flag" => StrategyRole::Flag,
            _ => StrategyRole::Table,
        }
    }

    /// Vertical layer for strategy map layout (0 = top).
    pub fn layout_layer(self) -> f32 {
        match self {
            StrategyRole::Limiter => 0.0,
            StrategyRole::Logic => 1.0,
            StrategyRole::Target => 2.0,
            StrategyRole::Table => 2.5,
            StrategyRole::Actuator => 3.0,
            StrategyRole::Sensor => 3.5,
            StrategyRole::Constant => 4.0,
            StrategyRole::Flag => 4.0,
        }
    }
}

/// Extract `wb-meta: key=val; key2=a,b` from description or title.
pub fn parse_wideband_meta(text: &str) -> WidebandMeta {
    let mut meta = WidebandMeta::default();
    let haystack = text.replace('\n', " ");
    let idx = haystack.to_ascii_lowercase().find("wb-meta:");
    if idx.is_none() {
        return meta;
    }
    let after = &haystack[idx.unwrap() + "wb-meta:".len()..];
    for part in after.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (key, val) = match part.split_once('=') {
            Some((k, v)) => (k.trim().to_ascii_lowercase(), v.trim()),
            None => continue,
        };
        match key.as_str() {
            "manager" => meta.manager = Some(val.to_string()),
            "role" => meta.role = StrategyRole::from_token(val),
            "depends" => {
                meta.depends = val
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            "tags" => {
                meta.tags = val
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            _ => {}
        }
    }
    meta
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParamKind {
    Table,
    Constant,
    Flag,
}

#[derive(Clone, Debug)]
pub struct StrategyNode {
    pub id: String,
    pub title: String,
    pub kind: ParamKind,
    pub meta: WidebandMeta,
    pub table_idx: Option<usize>,
    pub constant_idx: Option<usize>,
    pub flag_idx: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct StrategyEdge {
    pub from_id: String,
    pub to_id: String,
}

#[derive(Clone, Debug)]
pub struct StrategyGraph {
    pub nodes: Vec<StrategyNode>,
    pub edges: Vec<StrategyEdge>,
    pub positions: HashMap<String, (f32, f32)>,
}

pub fn build_strategy_graph(xdf: &XdfDocument) -> StrategyGraph {
    let mut nodes = Vec::new();
    let mut id_set = HashSet::new();

    for (i, t) in xdf.tables.iter().enumerate() {
        let combined = format!("{} {}", t.title, t.description);
        let meta = t.wideband_meta.clone().merge(parse_wideband_meta(&combined));
        let id = t.id.clone();
        id_set.insert(id.clone());
        nodes.push(StrategyNode {
            id,
            title: t.title.clone(),
            kind: ParamKind::Table,
            meta,
            table_idx: Some(i),
            constant_idx: None,
            flag_idx: None,
        });
    }
    for (i, c) in xdf.constants.iter().enumerate() {
        let combined = format!("{} {}", c.title, "");
        let meta = c.wideband_meta.clone().merge(parse_wideband_meta(&combined));
        let id = c.id.clone();
        id_set.insert(id.clone());
        nodes.push(StrategyNode {
            id,
            title: c.title.clone(),
            kind: ParamKind::Constant,
            meta,
            table_idx: None,
            constant_idx: Some(i),
            flag_idx: None,
        });
    }
    for (i, f) in xdf.flags.iter().enumerate() {
        let combined = format!("{} {}", f.title, f.description);
        let meta = f.wideband_meta.clone().merge(parse_wideband_meta(&combined));
        let id = f.id.clone();
        id_set.insert(id.clone());
        nodes.push(StrategyNode {
            id,
            title: f.title.clone(),
            kind: ParamKind::Flag,
            meta,
            table_idx: None,
            constant_idx: None,
            flag_idx: Some(i),
        });
    }

    let mut edges = Vec::new();
    for node in &nodes {
        for dep in &node.meta.depends {
            if id_set.contains(dep) {
                edges.push(StrategyEdge {
                    from_id: dep.clone(),
                    to_id: node.id.clone(),
                });
            }
        }
    }

    let positions = layout_strategy_nodes(&nodes);
    StrategyGraph {
        nodes,
        edges,
        positions,
    }
}

pub fn enrich_document_strategy_meta(xdf: &mut XdfDocument) {
    for table in &mut xdf.tables {
        let combined = format!("{} {}", table.title, table.description);
        let parsed = parse_wideband_meta(&combined);
        let inferred = infer_sample_meta(&table.id, &table.title);
        table.wideband_meta = parsed.merge(inferred);
    }
    for constant in &mut xdf.constants {
        let parsed = parse_wideband_meta(&constant.title);
        let inferred = infer_sample_meta(&constant.id, &constant.title);
        constant.wideband_meta = parsed.merge(inferred);
        if constant.id.contains("rev") {
            constant.wideband_meta.role = StrategyRole::Limiter;
        }
    }
    for flag in &mut xdf.flags {
        let combined = format!("{} {}", flag.title, flag.description);
        let parsed = parse_wideband_meta(&combined);
        flag.wideband_meta = parsed;
        if flag.id.contains("vvt") {
            flag.wideband_meta.manager = Some("vvt".to_string());
            flag.wideband_meta.role = StrategyRole::Logic;
        }
        if flag.id.contains("launch") {
            flag.wideband_meta.manager = Some("boost".to_string());
            flag.wideband_meta.role = StrategyRole::Logic;
            flag.wideband_meta.depends.push("boost_target".to_string());
        }
    }
}

impl WidebandMeta {
    pub fn merge(mut self, other: WidebandMeta) -> Self {
        if self.manager.is_none() {
            self.manager = other.manager;
        }
        if self.role == StrategyRole::Table && other.role != StrategyRole::Table {
            self.role = other.role;
        }
        if self.depends.is_empty() {
            self.depends = other.depends;
        }
        if self.tags.is_empty() {
            self.tags = other.tags;
        }
        self
    }
}

fn layout_strategy_nodes(nodes: &[StrategyNode]) -> HashMap<String, (f32, f32)> {
    let mut by_layer: HashMap<u32, Vec<&StrategyNode>> = HashMap::new();
    for n in nodes {
        let layer = n.meta.role.layout_layer() as u32;
        by_layer.entry(layer).or_default().push(n);
    }
    let mut layers: Vec<u32> = by_layer.keys().cloned().collect();
    layers.sort();

    let mut positions = HashMap::new();
    for (layer_idx, layer) in layers.iter().enumerate() {
        let row = by_layer.get(layer).unwrap();
        let count = row.len().max(1) as f32;
        for (i, n) in row.iter().enumerate() {
            let x = (i as f32 + 0.5) / count;
            let y = layer_idx as f32 / (layers.len().max(1) as f32);
            positions.insert(n.id.clone(), (x, y));
        }
    }
    positions
}

/// Heuristic meta for sample tables when wb-meta is absent (data-driven stubs).
pub fn infer_sample_meta(table_id: &str, title: &str) -> WidebandMeta {
    let t = format!("{} {}", table_id, title).to_ascii_lowercase();
    let mut meta = WidebandMeta::default();
    if t.contains("boost") || t.contains("wastegate") {
        meta.manager = Some("boost".to_string());
        meta.role = if t.contains("target") {
            StrategyRole::Target
        } else if t.contains("wastegate") || t.contains("wg") {
            StrategyRole::Actuator
        } else {
            StrategyRole::Target
        };
    } else if t.contains("vvt") || t.contains("cam") || t.contains("valve") {
        meta.manager = Some("vvt".to_string());
        meta.role = StrategyRole::Actuator;
    } else if t.contains("ve") || t.contains("lambda") || t.contains("load") || t.contains("torque") {
        meta.manager = Some("load".to_string());
        meta.role = StrategyRole::Table;
    } else if t.contains("spark") {
        meta.manager = Some("load".to_string());
        meta.role = StrategyRole::Target;
        meta.depends.push("ve_main".to_string());
    }
    meta
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_wb_meta_tag() {
        let m = parse_wideband_meta(
            "Boost target map. wb-meta: manager=boost; role=target; depends=ve_main,wg_duty",
        );
        assert_eq!(m.manager.as_deref(), Some("boost"));
        assert_eq!(m.role, StrategyRole::Target);
        assert_eq!(m.depends, vec!["ve_main".to_string(), "wg_duty".to_string()]);
    }
}
