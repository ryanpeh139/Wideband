use std::collections::HashSet;
use std::fs;
use std::path::Path;

pub const DELTA_EPSILON: f64 = 0.0001;

pub fn cell_changed(value: f64, basis: f64) -> bool {
    (value - basis).abs() > DELTA_EPSILON
}

/// Engineering cell text with signed delta suffix, matching WinForms Calibrate behavior.
pub fn format_cell_display(value: f64, basis: f64) -> String {
    let text = format!("{:.1}", value);
    if !cell_changed(value, basis) {
        return text;
    }
    let delta = value - basis;
    let sign = if delta > 0.0 { "+" } else { "" };
    format!("{} ({}{:.1})", text, sign, delta)
}

pub fn param_matches_query(
    query: &str,
    title: &str,
    description: &str,
    id: &str,
    units: &str,
) -> bool {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    title.to_lowercase().contains(&q)
        || description.to_lowercase().contains(&q)
        || id.to_lowercase().contains(&q)
        || units.to_lowercase().contains(&q)
}

pub fn table_has_edits(values: &[Vec<f64>], stock: &[Vec<f64>]) -> bool {
    for (r, row) in values.iter().enumerate() {
        let stock_row = stock.get(r);
        for (c, &val) in row.iter().enumerate() {
            let basis = stock_row
                .and_then(|sr| sr.get(c))
                .copied()
                .unwrap_or(val);
            if cell_changed(val, basis) {
                return true;
            }
        }
    }
    false
}

#[derive(Clone, Debug, Default)]
pub struct FavoriteTables {
    pub ids: HashSet<String>,
}

impl FavoriteTables {
    pub fn default_path() -> std::path::PathBuf {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                return dir.join("wideband-favorites.json");
            }
        }
        std::path::PathBuf::from("wideband-favorites.json")
    }

    pub fn load(path: &Path) -> Self {
        let raw = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => return Self::default(),
        };
        let ids: HashSet<String> = serde_json::from_str::<FavoritesFile>(&raw)
            .map(|f| f.table_ids.into_iter().collect())
            .unwrap_or_default();
        Self { ids }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let file = FavoritesFile {
            table_ids: self.ids.iter().cloned().collect(),
        };
        let json = serde_json::to_string_pretty(&file)
            .map_err(|e| format!("favorites serialize: {}", e))?;
        fs::write(path, json).map_err(|e| format!("favorites write: {}", e))
    }

    pub fn toggle(&mut self, id: &str) -> bool {
        if self.ids.contains(id) {
            self.ids.remove(id);
            false
        } else {
            self.ids.insert(id.to_string());
            true
        }
    }

    pub fn contains(&self, id: &str) -> bool {
        self.ids.contains(id)
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct FavoritesFile {
    table_ids: Vec<String>,
}
