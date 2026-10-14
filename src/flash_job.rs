use crate::audit_log::FlashAuditLog;
use crate::flash_layout::{FlashLayoutMap, FlashSegment};
use crate::j2534::PassThruDevice;
use crate::protocols::EcuProfile;
use crate::uds::{ProgrammingSession, UdsClient};

pub struct StockReadResult {
    pub bytes: Vec<u8>,
    pub origin: i64,
}

pub struct FlashJob;

impl FlashJob {
    pub fn read_stock(
        device: &mut dyn PassThruDevice,
        channel: u32,
        profile: &EcuProfile,
        layout: &FlashLayoutMap,
        device_label: &str,
        audit: &FlashAuditLog,
        mut progress: impl FnMut(usize, usize),
    ) -> Result<StockReadResult, String> {
        let spec = &layout.profile(&profile.id)?.stock_read;
        let mut client = UdsClient::for_profile(device, channel, profile);
        client.tester_present(device, profile.diagnostic)?;
        let (p2, p2_star) = client.diagnostic_session_control(device, profile.read_session)?;
        client.set_timers(p2, p2_star);
        let bytes = client.read_range(
            device,
            profile,
            spec.address,
            spec.size as usize,
            profile.max_chunk,
            progress,
        )?;
        let _ = audit.record_write(
            "stock_read",
            &profile.id,
            device_label,
            spec.address,
            spec.size,
            None,
            "ok",
            "full stock read via UDS 0x23",
        );
        Ok(StockReadResult {
            bytes,
            origin: layout.profile(&profile.id)?.rom_origin,
        })
    }

    pub fn flash_cal_segments(
        device: &mut dyn PassThruDevice,
        channel: u32,
        profile: &EcuProfile,
        layout: &FlashLayoutMap,
        image: &[u8],
        image_origin: i64,
        device_label: &str,
        audit: &FlashAuditLog,
        mut progress: impl FnMut(f32, &str),
    ) -> Result<(), String> {
        let segments = layout.cal_segments(&profile.id)?;
        let mut client = UdsClient::for_profile(device, channel, profile);
        ProgrammingSession::unlock(&mut client, device, profile)?;

        let total_segments = segments.len();
        for (idx, seg) in segments.iter().enumerate() {
            let slice = extract_segment(image, image_origin, seg)?;
            progress(
                idx as f32 / total_segments as f32,
                &format!("Flashing {} @ 0x{:X}", seg.label, seg.address),
            );
            crate::protocols::FlashDownload::execute_with_client(
                &mut client,
                device,
                channel,
                profile,
                seg.address as u32,
                &slice,
                |sent, total| {
                    let frac = (idx as f32 + (sent as f32 / total as f32)) / total_segments as f32;
                    progress(frac, &format!("{} {:.0}%", seg.label, (sent as f32 / total as f32) * 100.0));
                },
            )?;
            let _ = audit.record_write(
                "cal_flash",
                &profile.id,
                device_label,
                seg.address,
                seg.size,
                Some(&seg.id),
                "ok",
                &format!("segment {} complete", seg.label),
            );
        }
        progress(1.0, "Flash complete");
        Ok(())
    }
}

fn extract_segment(image: &[u8], image_origin: i64, seg: &FlashSegment) -> Result<Vec<u8>, String> {
    let start = seg.address as i64 - image_origin;
    if start < 0 {
        return Err(format!(
            "Segment 0x{:X} below image origin 0x{:X}",
            seg.address,
            image_origin
        ));
    }
    let start = start as usize;
    let end = start + seg.size as usize;
    if end > image.len() {
        return Err(format!(
            "Segment {} ({} bytes @ 0x{:X}) exceeds image buffer ({} bytes)",
            seg.id,
            seg.size,
            seg.address,
            image.len()
        ));
    }
    Ok(image[start..end].to_vec())
}
