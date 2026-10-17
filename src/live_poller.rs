//! Background cable worker: live-data polling, battery voltage, and the idle TesterPresent run
//! on this thread so the window only draws. Each cable request goes through J2534Host and takes
//! ~30 ms, so doing them on the UI thread made the window redraw a few times a second.
//!
//! The device is shared with the UI. The worker holds it for one request at a time; one-off UI
//! actions (Identify, DTC read, flash) hold it for their whole exchange, so they never interleave
//! with live polls.

use crate::j2534::PassThruDevice;
use crate::protocols::{EcuProfile, IsoTpLink};
use crate::telemetry::{decode_channel_value, TelemetryBuffer};
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

pub struct LivePoller {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl LivePoller {
    /// `keepalive` sends UDS `3E 80` every 2 s (real cables only, not the virtual bench).
    pub fn start(
        device: SharedDevice,
        telemetry: Arc<Mutex<TelemetryBuffer>>,
        profile: EcuProfile,
        status: Arc<LiveStatus>,
        keepalive: bool,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = stop.clone();
        let handle = std::thread::Builder::new()
            .name("live-poller".into())
            .spawn(move || run(device, telemetry, profile, status, keepalive, stop_flag))
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
    profile: EcuProfile,
    status: Arc<LiveStatus>,
    keepalive: bool,
    stop: Arc<AtomicBool>,
) {
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
            if let IdleTesterOutcome::Failed = keeper.tick(dev.as_mut(), &profile, true, !keepalive, false) {
                status.say(
                    "TesterPresent was not answered; idle keepalive stopped (channel stays open). Reconnect to retry."
                        .to_string(),
                );
            }
        }
        let_ui_in();

        if status.enabled.load(Ordering::SeqCst) {
            poll_channels(&device, &telemetry, &profile, &status, &stop);
        }
    }
}

fn poll_channels(
    device: &SharedDevice,
    telemetry: &Arc<Mutex<TelemetryBuffer>>,
    profile: &EcuProfile,
    status: &LiveStatus,
    stop: &AtomicBool,
) {
    let now = Instant::now();
    let poll_list: Vec<(usize, Vec<u8>, String, String)> = {
        let mut buf = lock(telemetry);
        let plan = buf.scheduler.plan(&buf.channels, now);
        buf.scheduler.apply_budget_skips(&plan.budget_skipped, now);
        plan.poll_indices
            .into_iter()
            .map(|idx| {
                let ch = &buf.channels[idx];
                (idx, ch.build_request_bytes(), ch.decode.clone(), ch.label.clone())
            })
            .collect()
    };

    for (idx, req, decode_id, label) in poll_list {
        if stop.load(Ordering::SeqCst) || status.paused.load(Ordering::SeqCst) {
            return;
        }
        let resp = {
            let mut slot = lock(device);
            let Some(dev) = slot.as_mut() else { return };
            let channel = dev.channel_id();
            IsoTpLink::exchange_for_profile(dev.as_mut(), channel, profile, &req, LIVE_REPLY_MS)
        };
        let_ui_in();
        let value = resp.as_ref().ok().and_then(|r| decode_channel_value(&decode_id, r));
        let mut buf = lock(telemetry);
        if let Some(value) = value {
            buf.set_channel_value(idx, value, now);
            buf.scheduler.record_success(idx, now);
            continue;
        }
        buf.bus_fail_count += 1;
        let refused = matches!(&resp, Ok(r) if r.len() >= 3 && r[0] == 0x7F && NRC_NOT_SUPPORTED.contains(&r[2]));
        let dropped = if refused {
            buf.scheduler.record_failure(idx, now);
            buf.scheduler.mark_unsupported(idx);
            true
        } else {
            buf.scheduler.record_failure(idx, now)
        };
        if dropped {
            let why = match &resp {
                Ok(r) => r.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" "),
                Err(e) => e.clone(),
            };
            let request = req.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ");
            status.say(format!(
                "Live data: {} ({}) is not supported by this ECU ({}); stopped polling it. Clear live to retry.",
                label, request, why
            ));
        }
    }
    lock(telemetry).push_frame();
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
        let mut poller = LivePoller::start(device.clone(), telemetry.clone(), profile.clone(), status.clone(), true);

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
