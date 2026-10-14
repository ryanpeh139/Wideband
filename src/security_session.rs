//! UDS/KWP diagnostic session control (0x10) and security access (0x27).
//! Key bytes come only from `security_config` (fail-closed; no guessed production masks).

use crate::j2534::PassThruDevice;
use crate::protocols::EcuProfile;
use crate::security_config::PlatformSecurity;
use crate::uds::UdsClient;

/// Default security level: request seed `0x27 0x01`, send key `0x27 0x02`.
pub const DEFAULT_SECURITY_LEVEL: u8 = 0x01;

#[derive(Debug)]
pub struct SecurityAccessOutcome {
    pub already_unlocked: bool,
    pub seed: Vec<u8>,
}

pub struct ProgrammingSession;

impl ProgrammingSession {
    /// Tester present, programming session, then seed-key unlock for memory write.
    pub fn unlock(
        client: &mut UdsClient,
        device: &mut dyn PassThruDevice,
        profile: &EcuProfile,
    ) -> Result<(), String> {
        let _ = Self::unlock_with_level(client, device, profile, DEFAULT_SECURITY_LEVEL)?;
        Ok(())
    }

    pub fn unlock_with_level(
        client: &mut UdsClient,
        device: &mut dyn PassThruDevice,
        profile: &EcuProfile,
        level: u8,
    ) -> Result<SecurityAccessOutcome, String> {
        if level < 0x01 || level > 0x7D || level % 2 == 0 {
            return Err("Security level must be an odd value from 0x01 through 0x7D.".to_string());
        }
        client.tester_present(device, profile.diagnostic)?;
        let (p2, p2_star) =
            client.diagnostic_session_control(device, profile.programming_session)?;
        client.set_timers(p2, p2_star);
        security_access(client, device, profile, level)
    }

    /// Read / extended diagnostic session (UDS default `0x03`, KWP `0x81`).
    pub fn enter_read_session(
        client: &mut UdsClient,
        device: &mut dyn PassThruDevice,
        profile: &EcuProfile,
    ) -> Result<(), String> {
        client.tester_present(device, profile.diagnostic)?;
        let (p2, p2_star) = client.diagnostic_session_control(device, profile.read_session)?;
        client.set_timers(p2, p2_star);
        Ok(())
    }
}

/// Request seed, compute key via configured algorithm, send key.
pub fn security_access(
    client: &mut UdsClient,
    device: &mut dyn PassThruDevice,
    profile: &EcuProfile,
    level: u8,
) -> Result<SecurityAccessOutcome, String> {
    let seed_resp = client.request(device, &[0x27, level], 0x67)?;
    if seed_resp.len() < 2 || seed_resp[1] != level {
        return Err("Security seed response invalid".to_string());
    }
    let seed = seed_resp[2..].to_vec();
    if seed.iter().all(|&b| b == 0) {
        return Ok(SecurityAccessOutcome {
            already_unlocked: true,
            seed,
        });
    }

    let key = PlatformSecurity::compute_key(&seed, profile)?;
    let key_level = level + 1;
    let mut key_req = vec![0x27, key_level];
    key_req.extend_from_slice(&key);
    let key_resp = client.request(device, &key_req, 0x67)?;
    if key_resp.len() < 2 || key_resp[1] != key_level {
        return Err("Security key rejected".to_string());
    }

    Ok(SecurityAccessOutcome {
        already_unlocked: false,
        seed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flash_layout::FlashLayoutMap;
    use crate::j2534::{PassThruDevice, VirtualEcuSimulator, ISO15765};
    use crate::protocols::{EcuProfile, FlashDownload};
    use crate::security_config::PlatformSecurity;
    use std::path::PathBuf;

    fn install_test_security_config() {
        PlatformSecurity::install(&PathBuf::from("tests/security-config-test.json"));
    }

    fn open_tacoma_sim() -> (VirtualEcuSimulator, u32) {
        let mut sim = VirtualEcuSimulator::new();
        sim.open().expect("open");
        let ch = sim.connect(ISO15765, 0, 500_000).expect("connect");
        (sim, ch)
    }

    #[test]
    fn test_diagnostic_session_control_programming() {
        install_test_security_config();
        let (mut sim, ch) = open_tacoma_sim();
        let prof = EcuProfile::tacoma_can();
        let mut client = UdsClient::for_profile(&mut sim, ch, &prof);

        let (p2, p2_star) = client
            .diagnostic_session_control(&mut sim, prof.programming_session)
            .expect("0x10 programming");
        assert_eq!(prof.programming_session, 0x02);
        assert!(p2 > 0);
        assert!(p2_star > 0);
    }

    #[test]
    fn test_security_access_happy_path_tacoma() {
        install_test_security_config();
        PlatformSecurity::register_bench_keys_for_virtual();
        let (mut sim, ch) = open_tacoma_sim();
        let prof = EcuProfile::tacoma_can();
        let mut client = UdsClient::for_profile(&mut sim, ch, &prof);

        let outcome = ProgrammingSession::unlock_with_level(&mut client, &mut sim, &prof, 0x01)
            .expect("unlock");
        assert!(!outcome.already_unlocked);
        assert_eq!(outcome.seed.len(), 4);
    }

    #[test]
    fn test_security_access_fails_without_key_config() {
        install_test_security_config();
        let prof = EcuProfile::tacoma_kline();
        let seed = [0x12, 0x34, 0x56, 0x78];
        let err = PlatformSecurity::compute_key(&seed, &prof).expect_err("fail-closed");
        assert!(
            err.contains("No security key function"),
            "unexpected: {}",
            err
        );
    }

    #[test]
    fn test_security_access_fails_wrong_key() {
        install_test_security_config();
        PlatformSecurity::register_bench_keys_for_virtual();
        let (mut sim, ch) = open_tacoma_sim();
        let prof = EcuProfile::tacoma_can();
        let mut client = UdsClient::for_profile(&mut sim, ch, &prof);
        client
            .diagnostic_session_control(&mut sim, prof.programming_session)
            .expect("session");

        client.request(&mut sim, &[0x27, 0x01], 0x67).expect("seed");
        let bad = client.request(&mut sim, &[0x27, 0x02, 0x00, 0x00, 0x00, 0x00], 0x67);
        assert!(bad.is_err());
        let err = bad.unwrap_err();
        assert!(err.contains("0x35") || err.contains("NRC"), "{}", err);
    }

    #[test]
    fn test_flash_download_blocked_without_unlock() {
        install_test_security_config();
        let (mut sim, ch) = open_tacoma_sim();
        let prof = EcuProfile::tacoma_can();
        let mut client = UdsClient::for_profile(&mut sim, ch, &prof);
        let image = vec![0xAAu8; 16];
        let err = FlashDownload::execute_with_client(
            &mut client,
            &mut sim,
            ch,
            &prof,
            0x10000,
            &image,
            |_, _| {},
        );
        assert!(err.is_err());
        let msg = err.unwrap_err();
        assert!(
            msg.contains("0x33") || msg.contains("security"),
            "expected security denial: {}",
            msg
        );
    }

    #[test]
    fn test_read_session_transition() {
        install_test_security_config();
        let layout = FlashLayoutMap::load("tests/flash-layout-test.json").expect("layout");
        let (mut sim, ch) = open_tacoma_sim();
        let prof = EcuProfile::tacoma_can();
        let mut client = UdsClient::for_profile(&mut sim, ch, &prof);
        ProgrammingSession::enter_read_session(&mut client, &mut sim, &prof).expect("read session");
        let spec = &layout.profile(&prof.id).unwrap().stock_read;
        let chunk = client
            .read_memory(&mut sim, &prof, spec.address, 8)
            .expect("read after session");
        assert_eq!(chunk.len(), 8);
    }
}
