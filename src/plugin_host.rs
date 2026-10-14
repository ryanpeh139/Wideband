use crate::pack_paths::discover_plugin_roots;
use crate::protocols::PluginRefs;
use libloading::{Library, Symbol};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::CStr;
use std::fs;
use std::path::{Path, PathBuf};
use wideband_plugin_api::{
    WB_CAP_CHECKSUM, WB_OK, WB_PLUGIN_API_VERSION, WB_SYM_CHECKSUM_HOOK, WB_SYM_GET_DESCRIPTOR,
    WB_SYM_MANAGER_HOOK, WB_SYM_PATCH_HOOK, WB_SYM_PROTOCOL_HOOK, WbChecksumHookFn,
    WbGetDescriptorFn, WbManagerHookFn, WbPatchHookFn, WbPluginDescriptor, WbProtocolHookFn,
};

#[derive(Clone, Debug)]
pub struct PluginRegistry {
    pub load_note: String,
    pub loaded_ids: Vec<String>,
}

struct LoadedPlugin {
    library: Library,
    plugin_id: String,
    capabilities: u32,
    checksum_hook: Option<WbChecksumHookFn>,
    protocol_hook: Option<WbProtocolHookFn>,
    patch_hook: Option<WbPatchHookFn>,
    manager_hook: Option<WbManagerHookFn>,
}

static REGISTRY: std::sync::OnceLock<std::sync::Mutex<BTreeMap<String, LoadedPlugin>>> =
    std::sync::OnceLock::new();

fn registry() -> &'static std::sync::Mutex<BTreeMap<String, LoadedPlugin>> {
    REGISTRY.get_or_init(|| std::sync::Mutex::new(BTreeMap::new()))
}

#[derive(Debug, serde::Deserialize)]
struct PluginManifest {
    plugin_version: u32,
    id: String,
    library: String,
}

impl PluginRegistry {
    pub fn load_default() -> Self {
        registry().lock().unwrap().clear();
        let mut errors = Vec::new();
        let mut loaded = BTreeSet::new();

        for root in discover_plugin_roots() {
            if !root.is_dir() {
                continue;
            }
            let entries = match fs::read_dir(&root) {
                Ok(e) => e,
                Err(e) => {
                    errors.push(format!("{}: {}", root.display(), e));
                    continue;
                }
            };
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                match load_plugin_dir(&path) {
                    Ok(id) => {
                        loaded.insert(id);
                    }
                    Err(e) => errors.push(format!("{}: {}", path.display(), e)),
                }
            }
        }

        let loaded_ids = loaded.into_iter().collect::<Vec<_>>();
        let load_note = if errors.is_empty() {
            if loaded_ids.is_empty() {
                "No community plugins discovered.".to_string()
            } else {
                format!("Loaded {} plugin(s): {}.", loaded_ids.len(), loaded_ids.join(", "))
            }
        } else if loaded_ids.is_empty() {
            format!("Plugin load failed: {}", errors.join(" | "))
        } else {
            format!(
                "Loaded {} plugin(s) with warnings: {}",
                loaded_ids.len(),
                errors.join(" | ")
            )
        };

        Self {
            load_note,
            loaded_ids,
        }
    }

    pub fn is_loaded(plugin_id: &str) -> bool {
        registry().lock().unwrap().contains_key(plugin_id)
    }

    /// Fail-closed when a pack references a plugin id that is not loaded.
    pub fn require_refs(refs: &PluginRefs) -> Result<(), String> {
        let mut missing = Vec::new();
        for id in refs.required_ids() {
            if !Self::is_loaded(&id) {
                missing.push(id);
            }
        }
        if missing.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "Required plugin(s) not loaded or invalid: {}. Install under plugins/ or set WIDEBAND_PLUGIN_DIRS.",
                missing.join(", ")
            ))
        }
    }

    pub fn invoke_checksum_hook(
        plugin_id: &str,
        profile_id: &str,
        image: &[u8],
        origin: i64,
    ) -> Result<String, String> {
        let guard = registry().lock().unwrap();
        let plugin = guard
            .get(plugin_id)
            .ok_or_else(|| format!("Checksum plugin '{}' is not loaded.", plugin_id))?;
        if (plugin.capabilities & WB_CAP_CHECKSUM) == 0 {
            return Err(format!(
                "Plugin '{}' does not advertise checksum capability.",
                plugin_id
            ));
        }
        let hook = plugin
            .checksum_hook
            .ok_or_else(|| format!("Plugin '{}' missing wb_checksum_hook export.", plugin_id))?;
        let mut buf = vec![0i8; 256];
        let rc = unsafe {
            hook(
                std::ffi::CString::new(profile_id)
                    .map_err(|_| "profile id contains NUL".to_string())?
                    .as_ptr(),
                image.as_ptr(),
                image.len() as u32,
                origin,
                buf.as_mut_ptr(),
                buf.len() as u32,
            )
        };
        if rc < 0 {
            return Err(format!("Plugin '{}' checksum hook failed ({})", plugin_id, rc));
        }
        let msg = unsafe { CStr::from_ptr(buf.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        Ok(msg)
    }
}

fn load_plugin_dir(dir: &Path) -> Result<String, String> {
    let manifest_path = dir.join("plugin.json");
    let text = fs::read_to_string(&manifest_path)
        .map_err(|e| format!("plugin.json: {}", e))?;
    let manifest: PluginManifest =
        serde_json::from_str(&text).map_err(|e| format!("plugin.json parse: {}", e))?;
    if manifest.plugin_version != 1 {
        return Err(format!(
            "unsupported plugin_version {} (expected 1)",
            manifest.plugin_version
        ));
    }
    if manifest.id.trim().is_empty() {
        return Err("plugin id must not be empty".to_string());
    }

    let lib_path = resolve_library_path(dir, &manifest.library)?;
    let library = unsafe { Library::new(&lib_path) }
        .map_err(|e| format!("library {}: {}", lib_path.display(), e))?;

    let get_descriptor: Symbol<WbGetDescriptorFn> = unsafe {
        library
            .get(WB_SYM_GET_DESCRIPTOR)
            .map_err(|e| format!("wb_plugin_get_descriptor: {}", e))?
    };
    let mut desc = WbPluginDescriptor {
        api_version: 0,
        plugin_id: std::ptr::null(),
        capabilities: 0,
    };
    let rc = unsafe { get_descriptor(&mut desc) };
    if rc != WB_OK {
        return Err(format!("wb_plugin_get_descriptor returned {}", rc));
    }
    if desc.api_version != WB_PLUGIN_API_VERSION {
        return Err(format!(
            "plugin API version {} (expected {})",
            desc.api_version,
            WB_PLUGIN_API_VERSION
        ));
    }
    if desc.plugin_id.is_null() {
        return Err("plugin returned null plugin_id".to_string());
    }
    let reported_id = unsafe { CStr::from_ptr(desc.plugin_id) }
        .to_string_lossy()
        .into_owned();
    if reported_id != manifest.id {
        return Err(format!(
            "plugin id mismatch: manifest '{}' vs descriptor '{}'",
            manifest.id,
            reported_id
        ));
    }

    let checksum_hook = load_checksum_hook(&library);
    let protocol_hook = load_protocol_hook(&library);
    let patch_hook = load_patch_hook(&library);
    let manager_hook = load_manager_hook(&library);

    let plugin = LoadedPlugin {
        library,
        plugin_id: manifest.id.clone(),
        capabilities: desc.capabilities,
        checksum_hook,
        protocol_hook,
        patch_hook,
        manager_hook,
    };

    let mut guard = registry().lock().unwrap();
    if guard.contains_key(&manifest.id) {
        return Err(format!("duplicate plugin id '{}'", manifest.id));
    }
    guard.insert(manifest.id.clone(), plugin);
    Ok(manifest.id)
}

fn load_checksum_hook(library: &Library) -> Option<WbChecksumHookFn> {
    unsafe {
        library
            .get(WB_SYM_CHECKSUM_HOOK)
            .ok()
            .map(|sym: Symbol<WbChecksumHookFn>| *sym)
    }
}

fn load_protocol_hook(library: &Library) -> Option<WbProtocolHookFn> {
    unsafe {
        library
            .get(WB_SYM_PROTOCOL_HOOK)
            .ok()
            .map(|sym: Symbol<WbProtocolHookFn>| *sym)
    }
}

fn load_patch_hook(library: &Library) -> Option<WbPatchHookFn> {
    unsafe {
        library
            .get(WB_SYM_PATCH_HOOK)
            .ok()
            .map(|sym: Symbol<WbPatchHookFn>| *sym)
    }
}

fn load_manager_hook(library: &Library) -> Option<WbManagerHookFn> {
    unsafe {
        library
            .get(WB_SYM_MANAGER_HOOK)
            .ok()
            .map(|sym: Symbol<WbManagerHookFn>| *sym)
    }
}

fn resolve_library_path(plugin_dir: &Path, library: &str) -> Result<PathBuf, String> {
    let file = library_file_name(library);
    let candidates = [
        plugin_dir.join("bin").join(&file),
        plugin_dir.join(&file),
        plugin_dir.join("lib").join(&file),
    ];
    for path in candidates {
        if path.is_file() {
            return Ok(path);
        }
    }
    Err(format!(
        "library '{}' not found under {} (looked for {})",
        library,
        plugin_dir.display(),
        file
    ))
}

fn library_file_name(base: &str) -> String {
    if cfg!(windows) {
        format!("{}.dll", base)
    } else if cfg!(target_os = "macos") {
        format!("lib{}.dylib", base)
    } else {
        format!("lib{}.so", base)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn build_noop_plugin() -> Option<PathBuf> {
        let status = Command::new("cargo")
            .args(["build", "-p", "wideband-noop", "--locked"])
            .status();
        if status.is_err() || !status.unwrap().success() {
            return None;
        }
        let path = PathBuf::from("target/debug").join(library_file_name("wideband_noop"));
        if path.is_file() {
            Some(path)
        } else {
            None
        }
    }

    #[test]
    fn loads_noop_plugin_when_built() {
        let lib = match build_noop_plugin() {
            Some(p) => p,
            None => {
                eprintln!("skip loads_noop_plugin_when_built: could not build wideband-noop");
                return;
            }
        };
        let temp = std::env::temp_dir().join(format!(
            "wideband-plugin-test-{}",
            std::process::id()
        ));
        let plugin_dir = temp.join("wideband-noop");
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(plugin_dir.join("bin")).unwrap();
        fs::copy(&lib, plugin_dir.join("bin").join(library_file_name("wideband_noop"))).unwrap();
        fs::write(
            plugin_dir.join("plugin.json"),
            r#"{"plugin_version":1,"id":"wideband-noop","library":"wideband_noop"}"#,
        )
        .unwrap();

        std::env::set_var("WIDEBAND_PLUGIN_DIRS", temp.to_string_lossy().to_string());
        let reg = PluginRegistry::load_default();
        assert!(
            reg.loaded_ids.iter().any(|id| id == "wideband-noop"),
            "note: {}",
            reg.load_note
        );
        let refs = PluginRefs {
            checksum: Some("wideband-noop".to_string()),
            protocol: None,
            patch: None,
            manager: None,
        };
        assert!(PluginRegistry::require_refs(&refs).is_ok());
        let msg = PluginRegistry::invoke_checksum_hook("wideband-noop", "tacoma-can", &[0u8; 8], 0)
            .expect("hook");
        assert!(msg.contains("noop"));
        let _ = fs::remove_dir_all(&temp);
    }
}
