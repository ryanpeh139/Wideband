//! J2534 v04.04 pass-through logger (32-bit). An application loads this DLL in place of the
//! cable's own; every PassThru call is forwarded unchanged to the real DLL and written to
//! `j2534-sniff.log` beside this DLL: arguments, messages, filters, IOCTLs, status, duration.
//!
//! The real DLL is the path in `real-dll.txt` beside this DLL, or the XHorse Mini-VCI driver.

#![allow(non_snake_case, clippy::missing_safety_doc)]

use std::ffi::c_void;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

const DEFAULT_REAL_DLL: &str =
    r"C:\Program Files (x86)\Autokent MVCI MultiDriver X64\1.4.3\MVCI32.dll";
const MAX_DATA: usize = 4128;
const ERR_NOT_SUPPORTED: i32 = 0x01;
const ERR_FAILED: i32 = 0x07;
const ERR_TIMEOUT: i32 = 0x09;
const ERR_BUFFER_EMPTY: i32 = 0x10;

type Hmodule = *mut c_void;

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryW(name: *const u16) -> Hmodule;
    fn GetProcAddress(module: Hmodule, name: *const u8) -> *mut c_void;
    fn GetModuleHandleExW(flags: u32, address: *const u16, module: *mut Hmodule) -> i32;
    fn GetModuleFileNameW(module: Hmodule, buffer: *mut u16, size: u32) -> u32;
}

#[repr(C)]
pub struct PassThruMsg {
    protocol_id: u32,
    rx_status: u32,
    tx_flags: u32,
    timestamp: u32,
    data_size: u32,
    extra_data_index: u32,
    data: [u8; MAX_DATA],
}

#[repr(C)]
struct SConfig {
    parameter: u32,
    value: u32,
}

#[repr(C)]
struct SConfigList {
    num_of_params: u32,
    config_ptr: *mut SConfig,
}

type OpenFn = unsafe extern "system" fn(*const c_void, *mut u32) -> i32;
type IdFn = unsafe extern "system" fn(u32) -> i32;
type ConnectFn = unsafe extern "system" fn(u32, u32, u32, u32, *mut u32) -> i32;
type MsgsFn = unsafe extern "system" fn(u32, *mut PassThruMsg, *mut u32, u32) -> i32;
type StartPeriodicFn = unsafe extern "system" fn(u32, *const PassThruMsg, *mut u32, u32) -> i32;
type TwoIdFn = unsafe extern "system" fn(u32, u32) -> i32;
type StartFilterFn = unsafe extern "system" fn(
    u32,
    u32,
    *const PassThruMsg,
    *const PassThruMsg,
    *const PassThruMsg,
    *mut u32,
) -> i32;
type SetVoltageFn = unsafe extern "system" fn(u32, u32, u32) -> i32;
type ReadVersionFn = unsafe extern "system" fn(u32, *mut u8, *mut u8, *mut u8) -> i32;
type LastErrorFn = unsafe extern "system" fn(*mut u8) -> i32;
type IoctlFn = unsafe extern "system" fn(u32, u32, *mut c_void, *mut c_void) -> i32;

struct Real {
    open: Option<OpenFn>,
    close: Option<IdFn>,
    connect: Option<ConnectFn>,
    disconnect: Option<IdFn>,
    read: Option<MsgsFn>,
    write: Option<MsgsFn>,
    start_periodic: Option<StartPeriodicFn>,
    stop_periodic: Option<TwoIdFn>,
    start_filter: Option<StartFilterFn>,
    stop_filter: Option<TwoIdFn>,
    set_voltage: Option<SetVoltageFn>,
    read_version: Option<ReadVersionFn>,
    last_error: Option<LastErrorFn>,
    ioctl: Option<IoctlFn>,
}

unsafe impl Send for Real {}
unsafe impl Sync for Real {}

static REAL: OnceLock<Option<Real>> = OnceLock::new();
static LOG: Mutex<Option<File>> = Mutex::new(None);
static STARTED: OnceLock<Instant> = OnceLock::new();
static EMPTY_READS: AtomicU64 = AtomicU64::new(0);

fn own_dir() -> PathBuf {
    unsafe {
        let mut module: Hmodule = std::ptr::null_mut();
        // FROM_ADDRESS | UNCHANGED_REFCOUNT
        let marker = own_dir as *const u16;
        if GetModuleHandleExW(0x4 | 0x2, marker, &mut module) == 0 {
            return PathBuf::from(".");
        }
        let mut buf = [0u16; 1024];
        let n = GetModuleFileNameW(module, buf.as_mut_ptr(), buf.len() as u32) as usize;
        let path = PathBuf::from(String::from_utf16_lossy(&buf[..n.min(buf.len())]));
        path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."))
    }
}

fn log(text: &str) {
    let started = STARTED.get_or_init(Instant::now);
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    if let Ok(mut guard) = LOG.lock() {
        if guard.is_none() {
            let path = own_dir().join("j2534-sniff.log");
            *guard = OpenOptions::new().create(true).append(true).open(path).ok();
        }
        if let Some(file) = guard.as_mut() {
            let _ = writeln!(file, "{:>11.1} {}", ms, text);
            let _ = file.flush();
        }
    }
}

fn flush_empty_reads() {
    let n = EMPTY_READS.swap(0, Ordering::Relaxed);
    if n > 0 {
        log(&format!("   ({} empty reads)", n));
    }
}

unsafe fn sym<T: Copy>(module: Hmodule, name: &str) -> Option<T> {
    let mut c = name.as_bytes().to_vec();
    c.push(0);
    let p = GetProcAddress(module, c.as_ptr());
    if p.is_null() {
        None
    } else {
        Some(std::mem::transmute_copy::<*mut c_void, T>(&p))
    }
}

fn real() -> Option<&'static Real> {
    REAL.get_or_init(|| unsafe {
        let configured = std::fs::read_to_string(own_dir().join("real-dll.txt"))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let path = configured.unwrap_or_else(|| DEFAULT_REAL_DLL.to_string());
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let module = LoadLibraryW(wide.as_ptr());
        if module.is_null() {
            log(&format!("== could not load real DLL {}", path));
            return None;
        }
        log(&format!("== j2534-sniff forwarding to {}", path));
        Some(Real {
            open: sym(module, "PassThruOpen"),
            close: sym(module, "PassThruClose"),
            connect: sym(module, "PassThruConnect"),
            disconnect: sym(module, "PassThruDisconnect"),
            read: sym(module, "PassThruReadMsgs"),
            write: sym(module, "PassThruWriteMsgs"),
            start_periodic: sym(module, "PassThruStartPeriodicMsg"),
            stop_periodic: sym(module, "PassThruStopPeriodicMsg"),
            start_filter: sym(module, "PassThruStartMsgFilter"),
            stop_filter: sym(module, "PassThruStopMsgFilter"),
            set_voltage: sym(module, "PassThruSetProgrammingVoltage"),
            read_version: sym(module, "PassThruReadVersion"),
            last_error: sym(module, "PassThruGetLastError"),
            ioctl: sym(module, "PassThruIoctl"),
        })
    })
    .as_ref()
}

fn hex(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len() * 3);
    for (i, b) in data.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        s.push_str(&format!("{:02X}", b));
    }
    s
}

unsafe fn fmt_msg(msg: *const PassThruMsg) -> String {
    if msg.is_null() {
        return "null".to_string();
    }
    let m = &*msg;
    let len = (m.data_size as usize).min(MAX_DATA);
    format!(
        "proto={} rx=0x{:X} tx=0x{:X} ts={} size={} extra={} [{}]",
        m.protocol_id,
        m.rx_status,
        m.tx_flags,
        m.timestamp,
        m.data_size,
        m.extra_data_index,
        hex(&m.data[..len])
    )
}

unsafe fn c_text(buf: *const u8, max: usize) -> String {
    if buf.is_null() {
        return String::new();
    }
    let mut out = Vec::new();
    for i in 0..max {
        let b = *buf.add(i);
        if b == 0 {
            break;
        }
        out.push(b);
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn ioctl_name(id: u32) -> &'static str {
    match id {
        0x01 => "GET_CONFIG",
        0x02 => "SET_CONFIG",
        0x03 => "READ_VBATT",
        0x04 => "FIVE_BAUD_INIT",
        0x05 => "FAST_INIT",
        0x07 => "CLEAR_TX_BUFFER",
        0x08 => "CLEAR_RX_BUFFER",
        0x09 => "CLEAR_PERIODIC_MSGS",
        0x0A => "CLEAR_MSG_FILTERS",
        0x0B => "CLEAR_FUNCT_MSG_LOOKUP_TABLE",
        0x0C => "ADD_TO_FUNCT_MSG_LOOKUP_TABLE",
        0x0D => "DELETE_FROM_FUNCT_MSG_LOOKUP_TABLE",
        0x0E => "READ_PROG_VOLTAGE",
        _ => "vendor",
    }
}

fn config_name(id: u32) -> String {
    let name = match id {
        0x01 => "DATA_RATE",
        0x03 => "LOOPBACK",
        0x04 => "NODE_ADDRESS",
        0x05 => "NETWORK_LINE",
        0x06 => "P1_MIN",
        0x07 => "P1_MAX",
        0x08 => "P2_MIN",
        0x09 => "P2_MAX",
        0x0A => "P3_MIN",
        0x0B => "P3_MAX",
        0x0C => "P4_MIN",
        0x0D => "P4_MAX",
        0x0E => "W1",
        0x0F => "W2",
        0x10 => "W3",
        0x11 => "W4",
        0x12 => "W5",
        0x13 => "TIDLE",
        0x14 => "TINIL",
        0x15 => "TWUP",
        0x16 => "PARITY",
        0x17 => "BIT_SAMPLE_POINT",
        0x18 => "SYNC_JUMP_WIDTH",
        0x19 => "W0",
        0x1A => "T1_MAX",
        0x1B => "T2_MAX",
        0x1C => "T4_MAX",
        0x1D => "T5_MAX",
        0x1E => "ISO15765_BS",
        0x1F => "ISO15765_STMIN",
        0x20 => "DATA_BITS",
        0x21 => "FIVE_BAUD_MOD",
        0x22 => "BS_TX",
        0x23 => "STMIN_TX",
        0x24 => "T3_MAX",
        0x25 => "ISO15765_WFT_MAX",
        _ => "",
    };
    if name.is_empty() {
        format!("0x{:X}", id)
    } else {
        name.to_string()
    }
}

unsafe fn fmt_config(list: *const c_void) -> String {
    if list.is_null() {
        return "null".to_string();
    }
    let l = &*(list as *const SConfigList);
    if l.config_ptr.is_null() || l.num_of_params > 64 {
        return format!("{} params", l.num_of_params);
    }
    (0..l.num_of_params as usize)
        .map(|i| {
            let c = &*l.config_ptr.add(i);
            format!("{}={}", config_name(c.parameter), c.value)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[no_mangle]
pub unsafe extern "system" fn PassThruOpen(name: *const c_void, device_id: *mut u32) -> i32 {
    let Some(f) = real().and_then(|r| r.open) else { return ERR_FAILED };
    let t = Instant::now();
    let status = f(name, device_id);
    let id = if device_id.is_null() { 0 } else { *device_id };
    log(&format!("PassThruOpen -> {} device={} ({} ms)", status, id, t.elapsed().as_millis()));
    status
}

#[no_mangle]
pub unsafe extern "system" fn PassThruClose(device_id: u32) -> i32 {
    let Some(f) = real().and_then(|r| r.close) else { return ERR_FAILED };
    flush_empty_reads();
    let status = f(device_id);
    log(&format!("PassThruClose device={} -> {}", device_id, status));
    status
}

#[no_mangle]
pub unsafe extern "system" fn PassThruConnect(
    device_id: u32,
    protocol_id: u32,
    flags: u32,
    baud: u32,
    channel_id: *mut u32,
) -> i32 {
    let Some(f) = real().and_then(|r| r.connect) else { return ERR_FAILED };
    let status = f(device_id, protocol_id, flags, baud, channel_id);
    let ch = if channel_id.is_null() { 0 } else { *channel_id };
    log(&format!(
        "PassThruConnect device={} protocol={} flags=0x{:X} baud={} -> {} channel={}",
        device_id, protocol_id, flags, baud, status, ch
    ));
    status
}

#[no_mangle]
pub unsafe extern "system" fn PassThruDisconnect(channel_id: u32) -> i32 {
    let Some(f) = real().and_then(|r| r.disconnect) else { return ERR_FAILED };
    flush_empty_reads();
    let status = f(channel_id);
    log(&format!("PassThruDisconnect channel={} -> {}", channel_id, status));
    status
}

#[no_mangle]
pub unsafe extern "system" fn PassThruReadMsgs(
    channel_id: u32,
    msgs: *mut PassThruMsg,
    count: *mut u32,
    timeout: u32,
) -> i32 {
    let Some(f) = real().and_then(|r| r.read) else { return ERR_FAILED };
    let asked = if count.is_null() { 0 } else { *count };
    let t = Instant::now();
    let status = f(channel_id, msgs, count, timeout);
    let got = if count.is_null() { 0 } else { *count };
    if got == 0 && (status == 0 || status == ERR_TIMEOUT || status == ERR_BUFFER_EMPTY) {
        EMPTY_READS.fetch_add(1, Ordering::Relaxed);
        return status;
    }
    flush_empty_reads();
    log(&format!(
        "PassThruReadMsgs channel={} asked={} timeout={} -> {} got={} ({} ms)",
        channel_id,
        asked,
        timeout,
        status,
        got,
        t.elapsed().as_millis()
    ));
    if !msgs.is_null() {
        for i in 0..got.min(asked) as usize {
            log(&format!("   rx {}", fmt_msg(msgs.add(i))));
        }
    }
    status
}

#[no_mangle]
pub unsafe extern "system" fn PassThruWriteMsgs(
    channel_id: u32,
    msgs: *mut PassThruMsg,
    count: *mut u32,
    timeout: u32,
) -> i32 {
    let Some(f) = real().and_then(|r| r.write) else { return ERR_FAILED };
    flush_empty_reads();
    let asked = if count.is_null() { 0 } else { *count };
    let mut lines = Vec::new();
    if !msgs.is_null() {
        for i in 0..asked.min(64) as usize {
            lines.push(format!("   tx {}", fmt_msg(msgs.add(i))));
        }
    }
    let t = Instant::now();
    let status = f(channel_id, msgs, count, timeout);
    let sent = if count.is_null() { 0 } else { *count };
    log(&format!(
        "PassThruWriteMsgs channel={} count={} timeout={} -> {} sent={} ({} ms)",
        channel_id,
        asked,
        timeout,
        status,
        sent,
        t.elapsed().as_millis()
    ));
    for line in lines {
        log(&line);
    }
    status
}

#[no_mangle]
pub unsafe extern "system" fn PassThruStartPeriodicMsg(
    channel_id: u32,
    msg: *const PassThruMsg,
    msg_id: *mut u32,
    interval: u32,
) -> i32 {
    let Some(f) = real().and_then(|r| r.start_periodic) else { return ERR_NOT_SUPPORTED };
    flush_empty_reads();
    let text = fmt_msg(msg);
    let status = f(channel_id, msg, msg_id, interval);
    let id = if msg_id.is_null() { 0 } else { *msg_id };
    log(&format!(
        "PassThruStartPeriodicMsg channel={} interval={} -> {} id={}\n   tx {}",
        channel_id, interval, status, id, text
    ));
    status
}

#[no_mangle]
pub unsafe extern "system" fn PassThruStopPeriodicMsg(channel_id: u32, msg_id: u32) -> i32 {
    let Some(f) = real().and_then(|r| r.stop_periodic) else { return ERR_NOT_SUPPORTED };
    let status = f(channel_id, msg_id);
    log(&format!("PassThruStopPeriodicMsg channel={} id={} -> {}", channel_id, msg_id, status));
    status
}

#[no_mangle]
pub unsafe extern "system" fn PassThruStartMsgFilter(
    channel_id: u32,
    filter_type: u32,
    mask: *const PassThruMsg,
    pattern: *const PassThruMsg,
    flow_control: *const PassThruMsg,
    filter_id: *mut u32,
) -> i32 {
    let Some(f) = real().and_then(|r| r.start_filter) else { return ERR_FAILED };
    flush_empty_reads();
    let (m, p, fc) = (fmt_msg(mask), fmt_msg(pattern), fmt_msg(flow_control));
    let status = f(channel_id, filter_type, mask, pattern, flow_control, filter_id);
    let id = if filter_id.is_null() { 0 } else { *filter_id };
    let kind = match filter_type {
        1 => "PASS",
        2 => "BLOCK",
        3 => "FLOW_CONTROL",
        _ => "?",
    };
    log(&format!(
        "PassThruStartMsgFilter channel={} type={} -> {} id={}\n   mask    {}\n   pattern {}\n   flow    {}",
        channel_id, kind, status, id, m, p, fc
    ));
    status
}

#[no_mangle]
pub unsafe extern "system" fn PassThruStopMsgFilter(channel_id: u32, filter_id: u32) -> i32 {
    let Some(f) = real().and_then(|r| r.stop_filter) else { return ERR_FAILED };
    let status = f(channel_id, filter_id);
    log(&format!("PassThruStopMsgFilter channel={} id={} -> {}", channel_id, filter_id, status));
    status
}

#[no_mangle]
pub unsafe extern "system" fn PassThruSetProgrammingVoltage(device_id: u32, pin: u32, voltage: u32) -> i32 {
    let Some(f) = real().and_then(|r| r.set_voltage) else { return ERR_NOT_SUPPORTED };
    let status = f(device_id, pin, voltage);
    log(&format!(
        "PassThruSetProgrammingVoltage device={} pin={} voltage={} -> {}",
        device_id, pin, voltage, status
    ));
    status
}

#[no_mangle]
pub unsafe extern "system" fn PassThruReadVersion(
    device_id: u32,
    firmware: *mut u8,
    dll: *mut u8,
    api: *mut u8,
) -> i32 {
    let Some(f) = real().and_then(|r| r.read_version) else { return ERR_NOT_SUPPORTED };
    let status = f(device_id, firmware, dll, api);
    log(&format!(
        "PassThruReadVersion device={} -> {} firmware={:?} dll={:?} api={:?}",
        device_id,
        status,
        c_text(firmware, 80),
        c_text(dll, 80),
        c_text(api, 80)
    ));
    status
}

#[no_mangle]
pub unsafe extern "system" fn PassThruGetLastError(text: *mut u8) -> i32 {
    let Some(f) = real().and_then(|r| r.last_error) else { return ERR_NOT_SUPPORTED };
    let status = f(text);
    log(&format!("PassThruGetLastError -> {} {:?}", status, c_text(text, 80)));
    status
}

#[no_mangle]
pub unsafe extern "system" fn PassThruIoctl(
    channel_id: u32,
    ioctl_id: u32,
    input: *mut c_void,
    output: *mut c_void,
) -> i32 {
    let Some(f) = real().and_then(|r| r.ioctl) else { return ERR_NOT_SUPPORTED };
    flush_empty_reads();
    let before = match ioctl_id {
        0x02 => fmt_config(input),
        0x05 | 0x0C | 0x0D => fmt_msg(input as *const PassThruMsg),
        _ => String::new(),
    };
    let status = f(channel_id, ioctl_id, input, output);
    let after = match ioctl_id {
        0x01 => fmt_config(input),
        0x03 | 0x0E if !output.is_null() => format!("{} mV", *(output as *const u32)),
        0x05 => fmt_msg(output as *const PassThruMsg),
        _ => String::new(),
    };
    log(&format!(
        "PassThruIoctl channel={} {} (0x{:X}) -> {} {}{}{}",
        channel_id,
        ioctl_name(ioctl_id),
        ioctl_id,
        status,
        before,
        if !before.is_empty() && !after.is_empty() { " => " } else { "" },
        after
    ));
    status
}
