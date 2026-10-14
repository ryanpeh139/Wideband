//! No-op plugin: exports all capability symbols and succeeds without altering ROM data.

use std::ffi::CStr;
use std::os::raw::c_char;
use wideband_plugin_api::{
    WB_CAP_CHECKSUM, WB_CAP_MANAGER, WB_CAP_PATCH, WB_CAP_PROTOCOL, WB_ERR_ARGS,
    WB_ERR_UNSUPPORTED, WB_OK, WB_PLUGIN_API_VERSION, WbPluginDescriptor,
};

static PLUGIN_ID: &[u8] = b"wideband-noop\0";

#[no_mangle]
pub unsafe extern "C" fn wb_plugin_get_descriptor(out: *mut WbPluginDescriptor) -> i32 {
    if out.is_null() {
        return WB_ERR_ARGS;
    }
    out.write(WbPluginDescriptor {
        api_version: WB_PLUGIN_API_VERSION,
        plugin_id: PLUGIN_ID.as_ptr() as *const c_char,
        capabilities: WB_CAP_CHECKSUM | WB_CAP_PROTOCOL | WB_CAP_PATCH | WB_CAP_MANAGER,
    });
    WB_OK
}

fn write_msg(out_msg: *mut c_char, out_cap: u32, text: &str) -> i32 {
    if out_msg.is_null() || out_cap == 0 {
        return WB_ERR_ARGS;
    }
    let bytes = text.as_bytes();
    let max = (out_cap as usize).saturating_sub(1);
    let n = bytes.len().min(max);
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_msg as *mut u8, n);
        *out_msg.add(n) = 0;
    }
    n as i32
}

#[no_mangle]
pub unsafe extern "C" fn wb_checksum_hook(
    _profile_id: *const c_char,
    _image: *const u8,
    _image_len: u32,
    _origin: i64,
    out_msg: *mut c_char,
    out_cap: u32,
) -> i32 {
    write_msg(out_msg, out_cap, "noop checksum hook (built-in stamping unchanged)")
}

#[no_mangle]
pub unsafe extern "C" fn wb_protocol_hook(
    _profile_id: *const c_char,
    out_msg: *mut c_char,
    out_cap: u32,
) -> i32 {
    write_msg(out_msg, out_cap, "noop protocol hook")
}

#[no_mangle]
pub unsafe extern "C" fn wb_patch_hook(
    _profile_id: *const c_char,
    out_msg: *mut c_char,
    out_cap: u32,
) -> i32 {
    write_msg(out_msg, out_cap, "noop patch hook")
}

#[no_mangle]
pub unsafe extern "C" fn wb_manager_hook(
    manager_id: *const c_char,
    out_msg: *mut c_char,
    out_cap: u32,
) -> i32 {
    let id = if manager_id.is_null() {
        "(null)"
    } else {
        CStr::from_ptr(manager_id).to_str().unwrap_or("(invalid)")
    };
    write_msg(out_msg, out_cap, &format!("noop manager hook for {}", id))
}

#[no_mangle]
pub unsafe extern "C" fn wb_plugin_unsupported_entry() -> i32 {
    WB_ERR_UNSUPPORTED
}
