pub mod audit_log;
pub mod cal_editor;
pub mod checksum_routine;
pub mod definitions;
pub mod rom_image;
pub mod diagnostics;
pub mod ecu_bus;
pub mod flash_job;
pub mod flash_layout;
pub mod managers;
pub mod patch_recipe;
pub mod isotp;
pub mod live_trace;
pub mod j2534;
pub mod j2534_host;
pub mod definition_pack;
pub mod pack_paths;
pub mod plugin_host;
pub mod profile_pack;
pub mod protocols;
pub mod security_config;
pub mod security_session;
pub mod strategy_meta;
pub mod strategy_ui;
pub mod telemetry;
pub mod telemetry_playback;
pub mod surface_mesh;
pub mod table_edit;
pub mod simple_ux;
pub mod tcm;
pub mod tester_present;
pub mod theme;
pub mod transport;
pub mod ui;
pub mod uds;
pub mod ve_correction;

use eframe::egui;
use ui::WidebandApp;

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 880.0])
            .with_min_inner_size([880.0, 600.0])
            .with_title("Wideband Motorsport Studio — OEM Calibration & Diagnostics"),
        ..Default::default()
    };

    eframe::run_native(
        "Wideband Motorsport Studio",
        native_options,
        Box::new(|cc| {
            theme::apply_motorsport_theme(&cc.egui_ctx);
            theme::install_symbol_fallback_font(&cc.egui_ctx);
            Ok(Box::new(WidebandApp::new(cc)))
        }),
    )
}

// ===========================================================================
// Comprehensive Unit Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use j2534::{PassThruDevice, PassThruMsg, VirtualEcuSimulator, ISO15765};
    use protocols::{ChecksumEngine, EcuIdentifier, EcuProfile, FlashDownload};
    use security_config::PlatformSecurity;
    use std::path::PathBuf;
    use cal_editor::{cell_changed, format_cell_display, param_matches_query, table_has_edits};
    use definitions::{CuwFile, ScaleEquation, XdfDocument};
    use security_config::{CHECKSUM_NOT_CONFIGURED, CHECKSUM_UPDATED, CHECKSUM_VALID};

    #[test]
    fn test_cal_editor_delta_and_search() {
        assert!(!cell_changed(10.0, 10.0));
        assert!(cell_changed(10.1, 10.0));
        assert_eq!(format_cell_display(12.5, 10.0), "12.5 (+2.5)");
        assert_eq!(format_cell_display(10.0, 10.0), "10.0");
        assert!(param_matches_query("ve", "VE Table", "", "ve_main", "%"));
        assert!(!param_matches_query("spark", "VE Table", "", "ve_main", "%"));
        let stock = vec![vec![1.0, 2.0], vec![3.0, 4.0]];
        let cur = vec![vec![1.0, 2.5], vec![3.0, 4.0]];
        assert!(table_has_edits(&cur, &stock));
    }

    #[test]
    fn test_checksum_badge_reflects_rom_state() {
        let _guard = PlatformSecurity::test_serial_guard();
        install_test_security_config();
        let mut rom = vec![0x10, 0x20, 0x30, 0x40, 0x00, 0x00, 0x00, 0x00];
        let _ = ChecksumEngine::update_and_verify(&mut rom, "tacoma-can", 0);
        assert_eq!(
            PlatformSecurity::checksum_badge("tacoma-can", &rom, 0),
            CHECKSUM_VALID
        );
        rom[0] = 0x11;
        assert_eq!(
            PlatformSecurity::checksum_badge("tacoma-can", &rom, 0),
            CHECKSUM_UPDATED
        );
    }

    #[test]
    fn test_known_cable_dll_catalog_includes_mvci_and_vxdiag() {
        let paths = j2534::known_passthru_dll_paths();
        let joined = paths
            .iter()
            .map(|(_, _, p)| p.to_ascii_lowercase())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            joined.contains("mvci") && joined.contains("vxdiag"),
            "discovery fallbacks should list Mini-VCI and VXDIAG install paths"
        );
    }

    #[test]
    fn test_j2534_big_endian_frame_packing() {
        let mut msg = PassThruMsg::default();
        msg.protocol_id = ISO15765;

        // 0x7E8 per SAE J2534-1 must be Big-Endian: 00 00 07 E8
        msg.write_id(0x7E8);
        assert_eq!(msg.data[0], 0x00);
        assert_eq!(msg.data[1], 0x00);
        assert_eq!(msg.data[2], 0x07);
        assert_eq!(msg.data[3], 0xE8);
        assert_eq!(msg.read_id(), 0x7E8);

        // 29-bit CAN ID 0x18DAF110: 18 DA F1 10
        msg.write_id(0x18DAF110);
        assert_eq!(msg.data[0], 0x18);
        assert_eq!(msg.data[1], 0xDA);
        assert_eq!(msg.data[2], 0xF1);
        assert_eq!(msg.data[3], 0x10);
        assert_eq!(msg.read_id(), 0x18DAF110);
    }

    #[test]
    fn test_cuw_s_record_parsing() {
        // Sample Motorola S-records: S3 4-byte address records with valid 1's complement checksums
        // Line 1: S3 09 00 01 00 00 AA BB CC 7E
        // Count = 0x09 (9 bytes: 4 address + 3 data + 1 checksum)
        // Addr = 00010000, Data = AA BB CC
        // Checksum: !(0x09 + 0x00 + 0x01 + 0x00 + 0x00 + 0xAA + 0xBB + 0xCC) = !(0x027F) = !(0x7F) = 0x80
        // Let's create an exact valid S3 record:
        // Bytes: [0x08, 0x00, 0x01, 0x00, 0x00, 0x11, 0x22]
        // Sum = 0x08 + 0x00 + 0x01 + 0x00 + 0x00 + 0x11 + 0x22 = 0x3C
        // Checksum = 0xFF - 0x3C = 0xC3
        let s_text = "HDR TOYOTA CUW TEST\nS307000100001122C4\nS70500010000F9\n";
        let cuw = CuwFile::parse(s_text.as_bytes()).expect("CUW parsing should succeed");

        assert_eq!(cuw.origin, 0x10000);
        assert_eq!(cuw.image.len(), 2);
        assert_eq!(cuw.image[0], 0x11);
        assert_eq!(cuw.image[1], 0x22);
    }

    #[test]
    fn test_strategy_graph_and_patch_recipe_preview() {
        use crate::patch_recipe::PatchRecipe;
        use crate::strategy_meta::build_strategy_graph;
        let xdf = XdfDocument::sample();
        let graph = build_strategy_graph(&xdf);
        assert!(graph.nodes.len() >= 5);
        assert!(graph.edges.iter().any(|e| e.from_id == "ve_main"));

        let recipe =
            PatchRecipe::load(std::path::Path::new("sample-patch-recipe.json")).expect("sample recipe");
        let rom = vec![0xFFu8; 0x20000];
        let preview = recipe.preview(&rom, 0, &xdf).expect("preview");
        assert!(!preview.notes.is_empty());
    }

    #[test]
    fn test_xdf_bind_table_scalar_and_flag_from_rom() {
        let xdf_xml = std::fs::read_to_string("tests/fixtures/tacoma-bench-table.xdf")
            .expect("fixture xdf");
        let mut doc = XdfDocument::parse(&xdf_xml).expect("parse fixture");
        assert_eq!(doc.tables[0].x_axis.dim, 2);
        assert_eq!(doc.tables[0].address, 0x100);

        let mut rom = vec![0xFFu8; 0x200];
        // Z table 2x2 @ 0x100: 10, 20 / 30, 40
        rom[0x100] = 10;
        rom[0x101] = 20;
        rom[0x102] = 30;
        rom[0x103] = 40;
        // X axis @ 0x108: raw 8,9 => 800, 900 RPM
        rom[0x108] = 8;
        rom[0x109] = 9;
        // Scalar rev @ 0x120: raw 200 => 6400 RPM
        rom[0x120] = 200;
        // Flag @ 0x130 bit0 set
        rom[0x130] = 0x01;

        doc.bind_from_rom(&rom, 0);
        assert_eq!(doc.tables[0].values[0][0], 10.0);
        assert_eq!(doc.tables[0].values[1][1], 40.0);
        assert!((doc.tables[0].x_axis.labels[0] - 800.0).abs() < 1e-3);
        assert!((doc.constants[0].value - 6400.0).abs() < 1e-3);
        assert!(doc.flags[0].is_set);
    }

    #[test]
    fn test_flash_blocked_without_checksum_config() {
        install_test_security_config();
        let rom = vec![0x10u8, 0x20, 0x30, 0x40];
        let err = PlatformSecurity::ensure_flash_checksum("tacoma-kline", "Tacoma K-Line", &rom, 0)
            .expect_err("flash must be blocked");
        assert!(err.contains("No checksum range"));
        assert_eq!(
            PlatformSecurity::checksum_badge("tacoma-kline", &rom, 0),
            CHECKSUM_NOT_CONFIGURED
        );
    }

    #[test]
    fn test_xdf_parsing_and_scale_equation() {
        let xdf_xml = r#"
        <XDFFORMAT version="1.60">
            <XDFHEADER>
                <deftitle>Test XDF Definition</deftitle>
            </XDFHEADER>
            <XDFTABLE uniqueid="tbl_ve">
                <title>VE Table Test</title>
                <description>Volumetric Efficiency</description>
                <XDFAXIS id="x">
                    <dim>4</dim>
                    <units>RPM</units>
                    <MATH equation="X * 100" />
                </XDFAXIS>
                <XDFAXIS id="y">
                    <dim>3</dim>
                    <units>MAP</units>
                    <MATH equation="X * 10" />
                </XDFAXIS>
                <XDFAXIS id="z">
                    <units>%</units>
                    <MATH equation="X * 0.1" />
                </XDFAXIS>
            </XDFTABLE>
            <XDFCONSTANT uniqueid="rev_lim">
                <title>Rev Limiter</title>
                <units>RPM</units>
                <MATH equation="X * 32" />
            </XDFCONSTANT>
            <XDFFLAG uniqueid="vvt_flag">
                <title>VVT Enable</title>
                <mask>0x04</mask>
            </XDFFLAG>
        </XDFFORMAT>
        "#;

        let doc = XdfDocument::parse(xdf_xml).expect("XDF XML parse must succeed");
        assert_eq!(doc.title, "Test XDF Definition");
        assert_eq!(doc.tables.len(), 1);
        assert_eq!(doc.tables[0].title, "VE Table Test");
        assert_eq!(doc.tables[0].x_axis.dim, 4);
        assert_eq!(doc.tables[0].y_axis.dim, 3);
        assert_eq!(doc.constants.len(), 1);
        assert_eq!(doc.constants[0].title, "Rev Limiter");
        assert_eq!(doc.flags.len(), 1);
        assert_eq!(doc.flags[0].mask, 0x04);

        // Test math equation forward & invert
        let eq = ScaleEquation::parse("(X - 128) * 0.5");
        let eng = eq.evaluate(200.0);
        assert_eq!(eng, 36.0);
        let raw = eq.invert(36.0);
        assert!((raw - 200.0).abs() < 1e-3);
    }

    fn install_test_security_config() {
        let path = PathBuf::from("tests/security-config-test.json");
        PlatformSecurity::install(&path);
    }

    #[test]
    fn test_checksum_calculations() {
        let _guard = PlatformSecurity::test_serial_guard();
        install_test_security_config();

        let test_bytes = vec![0x01, 0x02, 0x03, 0x04, 0x05];
        let sum16 = ChecksumEngine::sum16(&test_bytes);
        assert_eq!(sum16, 15);

        let sum32 = ChecksumEngine::sum32(&test_bytes);
        assert_eq!(sum32, 15);

        let crc = ChecksumEngine::crc32(&test_bytes);
        assert_ne!(crc, 0);

        let mut rom = vec![0x10, 0x20, 0x30, 0x40, 0x00, 0x00, 0x00, 0x00];
        let res = ChecksumEngine::update_and_verify(&mut rom, "tacoma-can", 0);
        assert!(res.is_ok());
        let stamped_sum = u32::from_be_bytes([rom[4], rom[5], rom[6], rom[7]]);
        assert_eq!(stamped_sum, 0x10 + 0x20 + 0x30 + 0x40);

        let seed = [0x12, 0x34, 0x56, 0x78];
        let tacoma = EcuProfile::tacoma_can();
        let tacoma_key = PlatformSecurity::compute_key(&seed, &tacoma).expect("Tacoma key must succeed");
        assert_eq!(tacoma_key.len(), 4);

        let silverado = EcuProfile::silverado_can();
        let silverado_key = PlatformSecurity::compute_key(&seed, &silverado).expect("Silverado key must succeed");
        assert_eq!(silverado_key.len(), 4);

        let unconfigured_profile = EcuProfile {
            id: "unconfigured-test-profile".to_string(),
            name: "Unconfigured Test".to_string(),
            platform: "Test".to_string(),
            bus: protocols::VehicleBus::CanIsoTp,
            diagnostic: protocols::DiagnosticProtocol::Uds,
            bitrate: 500000,
            protocol_id: ISO15765,
            connect_flags: 0,
            extended_id: false,
            request_id: 0x7E0,
            response_id: 0x7E8,
            filter_mask: 0x7FF,
            tester: 0xF1,
            ecu: 0x10,
            max_chunk: 1024,
            address_bytes: 4,
            size_bytes: 4,
            programming_session: 0x02,
            read_session: 0x03,
            security_profile_id: "unconfigured-test-profile".to_string(),
            flash_layout_profile_id: "unconfigured-test-profile".to_string(),
            checksum_profile_id: "unconfigured-test-profile".to_string(),
            plugins: protocols::PluginRefs::default(),
        };
        let unconfigured = PlatformSecurity::compute_key(&seed, &unconfigured_profile);
        assert!(unconfigured.is_err());
    }

    #[test]
    fn test_flash_layout_and_audit_log() {
        use crate::audit_log::FlashAuditLog;
        use crate::flash_job::FlashJob;
        use crate::flash_layout::FlashLayoutMap;
        use std::fs;

        let _guard = PlatformSecurity::test_serial_guard();
        install_test_security_config();
        PlatformSecurity::register_bench_keys_for_virtual();

        let layout = FlashLayoutMap::load("tests/flash-layout-test.json").expect("layout");
        let audit_path = std::env::temp_dir().join("wideband-audit-test.jsonl");
        let _ = fs::remove_file(&audit_path);
        let audit = FlashAuditLog::new(&audit_path);

        let mut sim = VirtualEcuSimulator::new();
        sim.open().unwrap();
        let channel = sim.connect(ISO15765, 0, 500000).unwrap();
        let prof = EcuProfile::tacoma_can();

        let stock = FlashJob::read_stock(
            &mut sim,
            channel,
            &prof,
            &layout,
            "virtual-ecu",
            &audit,
            |_d, _t| {},
        )
        .expect("stock read");
        assert_eq!(stock.bytes.len(), 256);
        assert_eq!(stock.origin, 65536);

        let rom = vec![0xCCu8; 65536];
        let flash = FlashJob::flash_cal_segments(
            &mut sim,
            channel,
            &prof,
            &layout,
            &rom,
            65536,
            "virtual-ecu",
            &audit,
            |_f, _d| {},
            false,
        );
        assert!(flash.is_ok());
        let text = fs::read_to_string(&audit_path).expect("audit file");
        assert!(text.contains("\"operation\":\"stock_read\""));
        assert!(text.contains("\"operation\":\"cal_flash\""));
    }

    #[test]
    fn test_virtual_ecu_loopback_flash() {
        let _guard = PlatformSecurity::test_serial_guard();
        install_test_security_config();
        PlatformSecurity::register_bench_keys_for_virtual();

        let mut sim = VirtualEcuSimulator::new();
        sim.open().expect("Simulator open must succeed");
        let channel = sim.connect(ISO15765, 0, 500000).expect("Connect must succeed");

        let prof = EcuProfile::tacoma_can();

        // 1. Test ECU Identification
        let id = EcuIdentifier::query(&mut sim, channel, &prof, true).expect("ECU identify must succeed");
        assert_eq!(id.vin, j2534::VIRTUAL_TACOMA_VIN);
        assert_eq!(id.cal_id, j2534::VIRTUAL_TACOMA_CAL_ID);
        assert_eq!(id.cvn, j2534::VIRTUAL_TACOMA_CVN_HEX);

        // 2. Test End-to-End Flash Sequence (Security -> Download 0x34 -> Transfer 0x36 -> Exit 0x37 -> Reset 0x11)
        let flash_data = vec![0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x11, 0x22];
        let flash_result = FlashDownload::execute(
            &mut sim,
            channel,
            &prof,
            0x10000,
            &flash_data,
            |sent, total| {
                assert!(sent <= total);
            },
        );

        assert!(flash_result.is_ok(), "Flash sequence must complete without error");
        sim.close().expect("Simulator close must succeed");
    }

    #[test]
    fn test_virtual_ecu_tcm_identify() {
        use crate::tcm::{TcmIdentifier, TcmProfile};

        let mut sim = VirtualEcuSimulator::new();
        sim.open().unwrap();
        let channel = sim.connect(ISO15765, 0, 500000).unwrap();
        let prof = TcmProfile::tacoma_can_tcm();
        let id = TcmIdentifier::query(&mut sim, channel, &prof, true).expect("TCM identify");
        assert_eq!(id.cal_id, j2534::VIRTUAL_TACOMA_TCM_CAL_ID);
        sim.close().unwrap();
    }

    #[test]
    fn test_virtual_ecu_read_only_diagnostics() {
        use diagnostics::{decode_dtc_bytes, DiagnosticsReader};

        assert_eq!(decode_dtc_bytes(0x01, 0x71), "P0171");
        assert_eq!(decode_dtc_bytes(0x03, 0x00), "P0300");

        let mut sim = VirtualEcuSimulator::new();
        sim.open().unwrap();
        let channel = sim.connect(ISO15765, 0, 500000).unwrap();
        let prof = EcuProfile::tacoma_can();

        let snap = DiagnosticsReader::read(&mut sim, channel, &prof).expect("diagnostics read");
        assert!(snap.mil_on);
        assert!(snap.stored_dtcs.iter().any(|d| d.code == "P0171"));
        assert!(snap.pending_dtcs.iter().any(|d| d.code == "P0300"));
        assert!(!snap.readiness.monitors.is_empty());
        assert!(!snap.freeze_frames.is_empty());
        sim.close().unwrap();
    }

    #[test]
    fn test_virtual_kline_diagnostics_and_connect() {
        use diagnostics::DiagnosticsReader;
        use j2534::connect_for_profile;

        let mut sim = VirtualEcuSimulator::new();
        sim.open().unwrap();
        let prof = EcuProfile::tacoma_kline();
        let channel = connect_for_profile(&mut sim, &prof).expect("kline connect");
        assert_eq!(channel, 1);

        let snap = DiagnosticsReader::read(&mut sim, channel, &prof).expect("kline diagnostics");
        assert!(snap.stored_dtcs.iter().any(|d| d.code == "P0171"));
        sim.close().unwrap();
    }

    #[test]
    fn test_virtual_vpw_diagnostics() {
        use diagnostics::DiagnosticsReader;
        use j2534::connect_for_profile;

        let mut sim = VirtualEcuSimulator::new();
        sim.open().unwrap();
        let prof = EcuProfile::silverado_vpw();
        let channel = connect_for_profile(&mut sim, &prof).expect("vpw connect");
        assert_eq!(channel, 1);

        let snap = DiagnosticsReader::read(&mut sim, channel, &prof).expect("vpw diagnostics");
        assert!(snap.stored_dtcs.iter().any(|d| d.code == "P0171"));
        assert!(snap.transport_note.contains("VPW"));
        sim.close().unwrap();
    }
}
