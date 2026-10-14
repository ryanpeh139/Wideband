use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Clone, Debug, Deserialize)]
pub struct FlashSegment {
    pub id: String,
    pub label: String,
    pub address: u64,
    pub size: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct StockReadSpec {
    pub address: u64,
    pub size: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ProfileFlashLayout {
    pub rom_origin: i64,
    pub image_bytes: u64,
    pub stock_read: StockReadSpec,
    pub cal_flash_segments: Vec<FlashSegment>,
}

#[derive(Clone, Debug, Deserialize)]
struct FlashLayoutFile {
    profiles: HashMap<String, ProfileFlashLayout>,
}

pub struct FlashLayoutMap {
    profiles: HashMap<String, ProfileFlashLayout>,
}

impl FlashLayoutMap {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let text = fs::read_to_string(path.as_ref())
            .map_err(|e| format!("flash-layout.json read failed: {}", e))?;
        let file: FlashLayoutFile =
            serde_json::from_str(&text).map_err(|e| format!("flash-layout.json parse failed: {}", e))?;
        Ok(Self {
            profiles: file.profiles,
        })
    }

    pub fn default_path() -> std::path::PathBuf {
        std::path::PathBuf::from("flash-layout.json")
    }

    pub fn profile(&self, profile_id: &str) -> Result<&ProfileFlashLayout, String> {
        self.profiles
            .get(profile_id)
            .ok_or_else(|| format!("No flash layout for profile '{}'", profile_id))
    }

    pub fn cal_segments(&self, profile_id: &str) -> Result<Vec<FlashSegment>, String> {
        Ok(self.profile(profile_id)?.cal_flash_segments.clone())
    }
}
