//! Stable C ABI shared between `wideband` and community `cdylib` plugins.
//!
//! Plugins export `wb_plugin_get_descriptor` and optional capability entry points.
//! Wideband loads plugins fail-closed when a profile or definition pack references
//! a missing or incompatible plugin id.

use std::os::raw::c_char;

pub const WB_PLUGIN_API_VERSION: u32 = 1;

pub const WB_OK: i32 = 0;
pub const WB_ERR_API: i32 = -1;
pub const WB_ERR_ARGS: i32 = -2;
pub const WB_ERR_UNSUPPORTED: i32 = -3;
pub const WB_ERR_FAIL_CLOSED: i32 = -4;

pub const WB_CAP_CHECKSUM: u32 = 1;
pub const WB_CAP_PROTOCOL: u32 = 2;
pub const WB_CAP_PATCH: u32 = 4;
pub const WB_CAP_MANAGER: u32 = 8;

#[repr(C)]
pub struct WbPluginDescriptor {
    pub api_version: u32,
    pub plugin_id: *const c_char,
    pub capabilities: u32,
}

pub type WbGetDescriptorFn = unsafe extern "C" fn(*mut WbPluginDescriptor) -> i32;

/// Optional checksum extension. Built-in `security-config.json` stamping still runs first;
/// plugins may append validation or alternate routines when referenced by a pack.
pub type WbChecksumHookFn = unsafe extern "C" fn(
    profile_id: *const c_char,
    image: *const u8,
    image_len: u32,
    origin: i64,
    out_msg: *mut c_char,
    out_cap: u32,
) -> i32;

pub type WbProtocolHookFn = unsafe extern "C" fn(
    profile_id: *const c_char,
    out_msg: *mut c_char,
    out_cap: u32,
) -> i32;

pub type WbPatchHookFn = unsafe extern "C" fn(
    profile_id: *const c_char,
    out_msg: *mut c_char,
    out_cap: u32,
) -> i32;

pub type WbManagerHookFn = unsafe extern "C" fn(
    manager_id: *const c_char,
    out_msg: *mut c_char,
    out_cap: u32,
) -> i32;

pub const WB_SYM_GET_DESCRIPTOR: &[u8] = b"wb_plugin_get_descriptor\0";
pub const WB_SYM_CHECKSUM_HOOK: &[u8] = b"wb_checksum_hook\0";
pub const WB_SYM_PROTOCOL_HOOK: &[u8] = b"wb_protocol_hook\0";
pub const WB_SYM_PATCH_HOOK: &[u8] = b"wb_patch_hook\0";
pub const WB_SYM_MANAGER_HOOK: &[u8] = b"wb_manager_hook\0";
