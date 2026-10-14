use std::env;
use std::path::{Path, PathBuf};

/// User-writable Wideband data directory (`~/.wideband` or `%APPDATA%\\Wideband`).
pub fn user_data_dir() -> Option<PathBuf> {
    if let Ok(home) = env::var("HOME") {
        return Some(PathBuf::from(home).join(".wideband"));
    }
    if let Ok(appdata) = env::var("APPDATA") {
        return Some(PathBuf::from(appdata).join("Wideband"));
    }
    None
}

pub fn exe_parent() -> Option<PathBuf> {
    env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
}

fn extra_dirs_from_env(var: &str) -> Vec<PathBuf> {
    env::var(var)
        .ok()
        .map(|raw| {
            raw.split(';')
                .chain(raw.split(','))
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default()
}

fn push_unique(out: &mut Vec<PathBuf>, path: PathBuf) {
    if path.as_os_str().is_empty() {
        return;
    }
    if out.iter().any(|p| p == &path) {
        return;
    }
    out.push(path);
}

/// Shipped + community ECU profile JSON directories (first wins on duplicate profile ids).
pub fn discover_profile_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(parent) = exe_parent() {
        push_unique(&mut roots, parent.join("profiles"));
    }
    push_unique(&mut roots, PathBuf::from("profiles"));
    for dir in extra_dirs_from_env("WIDEBAND_PROFILE_DIRS") {
        push_unique(&mut roots, dir);
    }
    if let Some(user) = user_data_dir() {
        push_unique(&mut roots, user.join("profiles"));
    }
    roots
}

/// Community definition pack roots (each contains one subdirectory per pack).
pub fn discover_definition_pack_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(parent) = exe_parent() {
        push_unique(&mut roots, parent.join("definition-packs"));
    }
    push_unique(&mut roots, PathBuf::from("definition-packs"));
    for dir in extra_dirs_from_env("WIDEBAND_DEFINITION_PACK_DIRS") {
        push_unique(&mut roots, dir);
    }
    if let Some(user) = user_data_dir() {
        push_unique(&mut roots, user.join("definition-packs"));
    }
    roots
}

/// Plugin bundle roots (each contains one subdirectory per plugin with `plugin.json`).
pub fn discover_plugin_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(parent) = exe_parent() {
        push_unique(&mut roots, parent.join("plugins"));
    }
    push_unique(&mut roots, PathBuf::from("plugins"));
    for dir in extra_dirs_from_env("WIDEBAND_PLUGIN_DIRS") {
        push_unique(&mut roots, dir);
    }
    if let Some(user) = user_data_dir() {
        push_unique(&mut roots, user.join("plugins"));
    }
    roots
}

pub fn is_dir(path: &Path) -> bool {
    path.is_dir()
}
