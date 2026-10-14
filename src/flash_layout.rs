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
    /// Set when `flash-layout.json` could not be loaded; every lookup fails closed with it.
    load_error: Option<String>,
}

impl FlashLayoutMap {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let text = fs::read_to_string(path.as_ref())
            .map_err(|e| format!("flash-layout.json read failed: {}", e))?;
        let file: FlashLayoutFile =
            serde_json::from_str(&text).map_err(|e| format!("flash-layout.json parse failed: {}", e))?;
        Ok(Self {
            profiles: file.profiles,
            load_error: None,
        })
    }

    /// Empty map used when the file is missing: stock read / flash refuse with `reason`.
    pub fn unavailable(reason: String) -> Self {
        Self {
            profiles: HashMap::new(),
            load_error: Some(reason),
        }
    }

    pub fn load_error(&self) -> Option<&str> {
        self.load_error.as_deref()
    }

    pub fn default_path() -> std::path::PathBuf {
        crate::pack_paths::beside_exe_or_cwd("flash-layout.json")
    }

    pub fn profile(&self, profile_id: &str) -> Result<&ProfileFlashLayout, String> {
        if let Some(err) = &self.load_error {
            return Err(format!(
                "{} — put flash-layout.json beside wideband.exe (run scripts\\Package-WindowsRelease.ps1)",
                err
            ));
        }
        self.profiles
            .get(profile_id)
            .ok_or_else(|| format!("No flash layout for profile '{}'", profile_id))
    }

    pub fn cal_segments(&self, profile_id: &str) -> Result<Vec<FlashSegment>, String> {
        Ok(self.profile(profile_id)?.cal_flash_segments.clone())
    }
}
