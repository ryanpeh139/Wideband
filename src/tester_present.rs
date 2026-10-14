//! Periodic UDS/KWP **TesterPresent** (`0x3E`) while a real Pass-Thru session is held open.
//! Parity with C# `VehicleLink` (2 s interval, skipped while flash/read owns the channel).

use crate::j2534::PassThruDevice;
use crate::protocols::EcuProfile;
use crate::uds::UdsClient;
use std::time::{Duration, Instant};

/// Same default as C# `VehicleLink` / cable page copy.
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleTesterOutcome {
    /// Not armed, virtual bench, flashing, or interval not elapsed.
    Skipped,
    /// Positive `0x7E` response.
    Sent,
    /// Request failed; keeper disarms until next real connect.
    Failed,
}

/// Holds an idle diagnostic session on real hardware without blocking the UI thread.
#[derive(Debug)]
pub struct IdleTesterPresent {
    armed: bool,
    last_sent: Instant,
    interval: Duration,
}

impl IdleTesterPresent {
    pub fn new() -> Self {
        Self {
            armed: false,
            last_sent: Instant::now(),
            interval: DEFAULT_INTERVAL,
        }
    }

    pub fn arm_for_real_hardware(&mut self) {
        self.armed = true;
        self.last_sent = Instant::now();
    }

    pub fn disarm(&mut self) {
        self.armed = false;
    }

    pub fn is_armed(&self) -> bool {
        self.armed
    }

    pub fn interval(&self) -> Duration {
        self.interval
    }

    pub fn should_tick(
        &self,
        is_connected: bool,
        is_virtual_bench: bool,
        channel_busy: bool,
    ) -> bool {
        self.armed
            && is_connected
            && !is_virtual_bench
            && !channel_busy
            && self.last_sent.elapsed() >= self.interval
    }

    /// Non-blocking keepalive tick. Call from the UI frame loop (same thread as other I/O).
    pub fn tick(
        &mut self,
        device: &mut dyn PassThruDevice,
        profile: &EcuProfile,
        is_connected: bool,
        is_virtual_bench: bool,
        channel_busy: bool,
    ) -> IdleTesterOutcome {
        if !self.should_tick(is_connected, is_virtual_bench, channel_busy) {
            return IdleTesterOutcome::Skipped;
        }
        self.last_sent = Instant::now();
        let channel = device.channel_id();
        let mut client = UdsClient::for_profile(device, channel, profile);
        client.set_timers(2000, 5000);
        match client.tester_present(device, profile.diagnostic) {
            Ok(()) => IdleTesterOutcome::Sent,
            Err(_) => {
                self.armed = false;
                IdleTesterOutcome::Failed
            }
        }
    }

    #[cfg(test)]
    pub fn force_due(&mut self) {
        self.last_sent = Instant::now() - self.interval - Duration::from_millis(1);
    }
}

impl Default for IdleTesterPresent {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isotp::mock::ScriptCan;
    use crate::protocols::EcuProfile;

    #[test]
    fn idle_tester_skips_virtual_bench() {
        let mut keeper = IdleTesterPresent::new();
        keeper.arm_for_real_hardware();
        let mut dev = ScriptCan::new(0x7E0, 0x7E8);
        dev.enqueue_rx(&[0x02, 0x7E, 0x00, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA]);
        let prof = EcuProfile::tacoma_can();
        let outcome = keeper.tick(&mut dev, &prof, true, true, false);
        assert_eq!(outcome, IdleTesterOutcome::Skipped);
        assert!(keeper.is_armed());
    }

    #[test]
    fn idle_tester_sends_on_scripted_can() {
        let mut keeper = IdleTesterPresent::new();
        keeper.arm_for_real_hardware();
        keeper.force_due();
        let mut dev = ScriptCan::new(0x7E0, 0x7E8);
        dev.enqueue_rx(&[0x02, 0x7E, 0x00, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA]);
        let prof = EcuProfile::tacoma_can();
        let outcome = keeper.tick(&mut dev, &prof, true, false, false);
        assert_eq!(outcome, IdleTesterOutcome::Sent);
        assert!(keeper.is_armed());
    }

    #[test]
    fn idle_tester_disarms_on_failure() {
        let mut keeper = IdleTesterPresent::new();
        keeper.arm_for_real_hardware();
        keeper.force_due();
        let mut dev = ScriptCan::new(0x7E0, 0x7E8);
        // No scripted RX → timeout / failure
        let prof = EcuProfile::tacoma_can();
        let outcome = keeper.tick(&mut dev, &prof, true, false, false);
        assert_eq!(outcome, IdleTesterOutcome::Failed);
        assert!(!keeper.is_armed());
    }

    #[test]
    fn idle_tester_skips_while_channel_busy() {
        let mut keeper = IdleTesterPresent::new();
        keeper.arm_for_real_hardware();
        keeper.force_due();
        let mut dev = ScriptCan::new(0x7E0, 0x7E8);
        let prof = EcuProfile::tacoma_can();
        let outcome = keeper.tick(&mut dev, &prof, true, false, true);
        assert_eq!(outcome, IdleTesterOutcome::Skipped);
        assert!(keeper.is_armed());
    }
}
