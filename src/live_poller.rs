//! Background cable worker: live-data polling, battery voltage, and the idle TesterPresent run
//! on this thread so the window only draws. Each cable request goes through J2534Host and takes
//! ~30 ms, so doing them on the UI thread made the window redraw a few times a second.
//!
//! The device is shared with the UI. The worker holds it for one request at a time; one-off UI
//! actions (Identify, DTC read, flash) hold it for their whole exchange, so they never interleave
//! with live polls.

use crate::j2534::PassThruDevice;
use crate::protocols::{EcuProfile, IsoTpLink};
use crate::telemetry::{decode_channel_value, mode01_data_len, split_mode01_reply, ChannelModule, TelemetryBuffer};
use crate::tester_present::{IdleTesterOutcome, IdleTesterPresent};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub type SharedDevice = Arc<Mutex<Option<Box<dyn PassThruDevice>>>>;

/// How long one live-data poll waits for its reply. The ECM answers in ~15–30 ms through
/// J2534Host; a shorter wait left late replies in the cable buffer for the next request.
pub const LIVE_REPLY_MS: u32 = 100;
const VBATT_EVERY: Duration = Duration::from_secs(1);
/// Negative response codes meaning "this ECU does not have it": serviceNotSupported,
/// subFunctionNotSupported, requestOutOfRange.
const NRC_NOT_SUPPORTED: [u8; 3] = [0x11, 0x12, 0x31];

/// State the UI and the worker share.
#[derive(Default)]
pub struct LiveStatus {
    /// Live data on (the UI's live toggle).
    pub enabled: AtomicBool,
    /// A flash or stock read owns the bus.
    pub paused: AtomicBool,
    /// The J2534Host session died; the UI disconnects.
    pub session_lost: AtomicBool,
    pub vbatt: Mutex<Option<f32>>,
    messages: Mutex<Vec<String>>,
}

impl LiveStatus {
    pub fn take_messages(&self) -> Vec<String> {
        std::mem::take(&mut *lock(&self.messages))
    }

    fn say(&self, text: String) {
        lock(&self.messages).push(text);
    }
}

/// Who live data talks to: the engine ECU (the connected profile) and, for channels marked
/// `"module": "tcm"`, the transmission ECU on the same connection.
pub struct Profiles {
    pub ecm: EcuProfile,
    pub tcm: Option<EcuProfile>,
}

pub struct LivePoller {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl LivePoller {
    /// `keepalive` sends UDS `3E 80` every 2 s (real cables only, not the virtual bench).
    pub fn start(
        device: SharedDevice,
        telemetry: Arc<Mutex<TelemetryBuffer>>,
        profiles: Profiles,
        status: Arc<LiveStatus>,
        keepalive: bool,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = stop.clone();
        let handle = std::thread::Builder::new()
            .name("live-poller".into())
            .spawn(move || run(device, telemetry, profiles, status, keepalive, stop_flag))
            .ok();
        Self { stop, handle }
    }

    /// Stops the worker and waits for its current request to finish.
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for LivePoller {
    fn drop(&mut self) {
        self.stop();
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The device mutex is not fair: released and re-locked in a tight loop, the worker kept a
/// waiting UI action (Identify) out for ~800 ms on the truck. A short pause after each cable
/// exchange hands the device over.
fn let_ui_in() {
    std::thread::sleep(Duration::from_millis(1));
}

fn run(
    device: SharedDevice,
    telemetry: Arc<Mutex<TelemetryBuffer>>,
    profiles: Profiles,
    status: Arc<LiveStatus>,
    keepalive: bool,
    stop: Arc<AtomicBool>,
) {
    let mut batching = true;
    let mut failed_batches = 0u32;
    let mut keeper = IdleTesterPresent::new();
    if keepalive {
        keeper.arm_for_real_hardware();
    }
    let mut last_vbatt: Option<Instant> = None;
    let mut next_tick = Instant::now();

    while !stop.load(Ordering::SeqCst) {
        let now = Instant::now();
        if now < next_tick {
            std::thread::sleep((next_tick - now).min(Duration::from_millis(10)));
            continue;
        }
        next_tick = now + lock(&telemetry).scheduler.tick_interval();
        if status.paused.load(Ordering::SeqCst) {
            continue;
        }

        {
            let mut slot = lock(&device);
            let Some(dev) = slot.as_mut() else { continue };
            if !dev.is_connected() {
                status.session_lost.store(true, Ordering::SeqCst);
                return;
            }
            if last_vbatt.map_or(true, |t| now.duration_since(t) >= VBATT_EVERY) {
                last_vbatt = Some(now);
                let channel = dev.channel_id();
                if let Some(v) = crate::ui::read_pass_thru_vbatt(dev.as_mut(), channel) {
                    *lock(&status.vbatt) = Some(v);
                }
            }
            if let IdleTesterOutcome::Failed = keeper.tick(dev.as_mut(), &profiles.ecm, true, !keepalive, false) {
                status.say(
                    "TesterPresent was not answered; idle keepalive stopped (channel stays open). Reconnect to retry."
                        .to_string(),
                );
            }
        }
        let_ui_in();

        if status.enabled.load(Ordering::SeqCst) {
            poll_channels(&device, &telemetry, &profiles, &status, &stop, &mut batching, &mut failed_batches);
        }
    }
}

/// One channel due this tick.
struct Poll {
    idx: usize,
    req: Vec<u8>,
    decode: String,
    label: String,
    module: ChannelModule,
}

/// ECM Mode 01 channels whose reply length is known go out up to this many PIDs per request
/// (SAE J1979 allows six). Each cable round trip costs ~30 ms, so batching multiplies the rate.
const MAX_PIDS_PER_REQUEST: usize = 6;

fn batchable(p: &Poll) -> bool {
    p.module == ChannelModule::Ecm && p.req.len() == 2 && p.req[0] == 0x01 && mode01_data_len(p.req[1]).is_some()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ")
}

fn poll_channels(
    device: &SharedDevice,
    telemetry: &Arc<Mutex<TelemetryBuffer>>,
    profiles: &Profiles,
    status: &LiveStatus,
    stop: &AtomicBool,
    batching: &mut bool,
    failed_batches: &mut u32,
) {
    let now = Instant::now();
    let polls: Vec<Poll> = {
        let mut buf = lock(telemetry);
        let plan = buf.scheduler.plan(&buf.channels, now);
        buf.scheduler.apply_budget_skips(&plan.budget_skipped, now);
        plan.poll_indices
            .into_iter()
            .map(|idx| {
                let ch = &buf.channels[idx];
                Poll {
                    idx,
                    req: ch.build_request_bytes(),
                    decode: ch.decode.clone(),
                    label: ch.label.clone(),
                    module: ch.module,
                }
            })
            .collect()
    };

    let (mut batch, singles): (Vec<Poll>, Vec<Poll>) = if *batching {
        polls.into_iter().partition(batchable)
    } else {
        (Vec::new(), polls)
    };
    while !batch.is_empty() {
        if stop.load(Ordering::SeqCst) || status.paused.load(Ordering::SeqCst) {
            return;
        }
        let rest = batch.split_off(batch.len().min(MAX_PIDS_PER_REQUEST));
        let group = std::mem::replace(&mut batch, rest);
        if group.len() == 1 {
            poll_single(device, telemetry, profiles, status, group.into_iter().next().unwrap(), now);
            continue;
        }
        let mut req = vec![0x01];
        req.extend(group.iter().map(|p| p.req[1]));
        let resp = exchange(device, &profiles.ecm, &req);
        let parts = resp.as_ref().map(|r| split_mode01_reply(r)).unwrap_or_default();
        if parts.is_empty() {
            // No PID of this batch answered (all unsupported, or a lost reply): ask one PID at a
            // time this round. If single PIDs answer while batches keep failing, the ECU does not
            // take multi-PID requests, so stop batching.
            let mut answered = false;
            for p in group {
                answered |= poll_single(device, telemetry, profiles, status, p, now);
            }
            if answered {
                *failed_batches += 1;
                if *failed_batches >= 3 {
                    *batching = false;
                    status.say(format!(
                        "Live data: the ECU does not answer multi-PID requests ({} -> {}); polling one PID at a time.",
                        hex(&req),
                        match &resp {
                            Ok(r) => hex(r),
                            Err(e) => e.clone(),
                        }
                    ));
                }
            }
            continue;
        }
        *failed_batches = 0;
        for p in group {
            let single = parts.iter().find(|(pid, _)| *pid == p.req[1]).map(|(_, r)| r.clone());
            // A PID the ECU lacks is left out of a multi-PID reply rather than refused.
            let outcome = single.ok_or_else(|| "not in the multi-PID reply".to_string());
            record(telemetry, status, &p, &outcome, now);
        }
    }
    for p in singles {
        if stop.load(Ordering::SeqCst) || status.paused.load(Ordering::SeqCst) {
            return;
        }
        poll_single(device, telemetry, profiles, status, p, now);
    }
    lock(telemetry).push_frame();
}

fn poll_single(
    device: &SharedDevice,
    telemetry: &Arc<Mutex<TelemetryBuffer>>,
    profiles: &Profiles,
    status: &LiveStatus,
    p: Poll,
    now: Instant,
) -> bool {
    let resp = match p.module {
        ChannelModule::Ecm => exchange(device, &profiles.ecm, &p.req),
        ChannelModule::Tcm => match &profiles.tcm {
            Some(tcm) => exchange(device, tcm, &p.req),
            None => Err("no transmission profile is selected".to_string()),
        },
    };
    record(telemetry, status, &p, &resp, now)
}

fn exchange(device: &SharedDevice, profile: &EcuProfile, req: &[u8]) -> Result<Vec<u8>, String> {
    let resp = {
        let mut slot = lock(device);
        let Some(dev) = slot.as_mut() else {
            return Err("cable closed".to_string());
        };
        let channel = dev.channel_id();
        IsoTpLink::exchange_for_profile(dev.as_mut(), channel, profile, req, LIVE_REPLY_MS)
    };
    let_ui_in();
    resp
}

/// Stores a decoded value (returns true), or counts a failure and drops the channel once it is
/// clearly unsupported.
fn record(
    telemetry: &Arc<Mutex<TelemetryBuffer>>,
    status: &LiveStatus,
    p: &Poll,
    resp: &Result<Vec<u8>, String>,
    now: Instant,
) -> bool {
    let value = resp.as_ref().ok().and_then(|r| decode_channel_value(&p.decode, r));
    let mut buf = lock(telemetry);
    if let Some(value) = value {
        buf.set_channel_value(p.idx, value, now);
        buf.scheduler.record_success(p.idx, now);
        return true;
    }
    buf.bus_fail_count += 1;
    let refused = matches!(resp, Ok(r) if r.len() >= 3 && r[0] == 0x7F && NRC_NOT_SUPPORTED.contains(&r[2]));
    let dropped = if refused {
        buf.scheduler.record_failure(p.idx, now);
        buf.scheduler.mark_unsupported(p.idx);
        true
    } else {
        buf.scheduler.record_failure(p.idx, now)
    };
    if dropped {
        let why = match resp {
            Ok(r) => hex(r),
            Err(e) => e.clone(),
        };
        let module = match p.module {
            ChannelModule::Ecm => "ECU",
            ChannelModule::Tcm => "transmission ECU",
        };
        status.say(format!(
            "Live data: {} ({}) is not supported by this {} ({}); stopped polling it. Clear live to retry.",
            p.label,
            hex(&p.req),
            module,
            why
        ));
    }
    false
}


/// Read-only bench check with a real cable and ECM (ignition ON). Skipped unless
/// `WIDEBAND_HW_DLL` names the J2534 DLL.
#[cfg(all(test, windows))]
mod hardware {
    use super::*;
    use crate::protocols::EcuIdentifier;
    use crate::telemetry::TelemetryChannelFile;

    #[test]
    #[ignore]
    fn hardware_worker_keeps_ui_free_and_polls_every_channel() {
        let dll = match std::env::var("WIDEBAND_HW_DLL") {
            Ok(path) => path,
            Err(_) => return,
        };
        let profile = EcuProfile::tacoma_can();
        let mut dev = crate::j2534_host::create_host_device(&dll);
        dev.open().expect("open cable");
        crate::j2534::connect_for_profile(dev.as_mut(), &profile).expect("connect");
        let device: SharedDevice = Arc::new(Mutex::new(Some(dev)));
        let def = TelemetryChannelFile::load(std::path::Path::new("telemetry-channels.json")).expect("channels");
        let telemetry = Arc::new(Mutex::new(TelemetryBuffer::from_definition(&def, None)));
        let status = Arc::new(LiveStatus::default());
        status.enabled.store(true, Ordering::SeqCst);
        let profiles = Profiles { ecm: profile.clone(), tcm: Some(crate::tcm::TcmProfile::tacoma_can_tcm().to_diagnostic_profile()) };
        let mut poller = LivePoller::start(device.clone(), telemetry.clone(), profiles, status.clone(), true);

        // A 60 fps window only touches the telemetry buffer; measure its worst wait.
        let started = Instant::now();
        let mut worst_frame = Duration::ZERO;
        while started.elapsed() < Duration::from_secs(4) {
            let t = Instant::now();
            let _ = lock(&telemetry).value("rpm");
            worst_frame = worst_frame.max(t.elapsed());
            std::thread::sleep(Duration::from_millis(16));
        }
        let t = Instant::now();
        let id = {
            let mut slot = lock(&device);
            let dev = slot.as_mut().expect("device");
            let channel = dev.channel_id();
            EcuIdentifier::query(dev.as_mut(), channel, &profile, false)
        };
        let identify_ms = t.elapsed().as_millis();
        std::thread::sleep(Duration::from_secs(1));
        poller.stop();
        if let Some(mut dev) = lock(&device).take() {
            let _ = dev.close();
        }

        let buf = lock(&telemetry);
        for (idx, ch) in buf.channels.iter().enumerate() {
            println!(
                "{:<18} {:>5.1} Hz  value={:<10.2} {}",
                ch.id,
                buf.channel_rates[idx].hz,
                buf.latest[idx],
                if buf.scheduler.is_unsupported(idx) { "unsupported" } else { "" }
            );
        }
        for m in status.take_messages() {
            println!("msg: {}", m);
        }
        println!("worst UI wait {:?}; Identify while live {} ms; vbatt {:?}", worst_frame, identify_ms, *lock(&status.vbatt));
        let id = id.expect("identify while live data runs");
        println!("{}", id.details);
        assert_eq!(id.vin.len(), 17);
        assert!(worst_frame < Duration::from_millis(20), "UI waited {:?}", worst_frame);
        let rpm = buf.channels.iter().position(|c| c.id == "rpm").unwrap();
        assert!(buf.channel_rates[rpm].hz > 2.0, "rpm {} Hz", buf.channel_rates[rpm].hz);
    }
}
