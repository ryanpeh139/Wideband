use crate::audit_log::FlashAuditLog;
use crate::cal_editor::{
    cell_changed, format_cell_display, param_matches_query, table_has_edits, FavoriteTables,
};
use crate::definition_pack::DefinitionPackCatalog;
use crate::definitions::{RomImage, XdfDocument};
use crate::plugin_host::PluginRegistry;
use crate::diagnostics::{
    self, DiagnosticsSnapshot, DtcStatus, status_label,
};
use crate::flash_job::FlashJob;
use crate::flash_layout::FlashLayoutMap;
use crate::ecu_update::{
    check_update_file, compare_snapshots, readiness, CheckLevel, PayloadKind, UpdateComparison,
    UpdateFileCheck, UpdateSnapshot,
};
use crate::j2534::{connect_for_profile, PassThruDevice, READ_VBATT};
use crate::transport::{TransportAvailability, TransportDevice, TransportRegistry};
use crate::protocols::{EcuIdentification, EcuIdentifier, EcuProfile};
use crate::tcm::{
    build_parameter_view, flash_blockers as tcm_flash_blockers, group_label, TcmIdentifier,
    TcmParameterGroup, TcmParameterView, TcmProfile, TcmProfileCatalog,
};
use crate::security_config::{
    key_registration_label, security_mode_label, PlatformSecurity, KeyRegistration, SecurityMode,
    CHECKSUM_NOT_CONFIGURED, CHECKSUM_VALID,
};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use crate::simple_ux::SimpleUxState;
use crate::live_trace;
use crate::surface_mesh;
use crate::table_edit::{self, CellSelection, MathOp, UndoStack};
use crate::ve_correction::{self, CorrectionResult, CorrectionSettings, FrameSource, TargetLambda};
use crate::telemetry::{decode_channel_value, TelemetryBuffer, TelemetryChannelFile, TelemetryFrame};
use crate::telemetry_playback::{
    import_log_csv, ImportedLog, LogMarker, MathChannelDef, TelemetryPlaybackState,
    TelemetryViewMode,
};
use crate::tester_present::{IdleTesterOutcome, IdleTesterPresent};
use crate::theme::{
    self, paint_live_cell_glow, paint_studio_badge, studio_tab_button, ACCENT_CYAN, CARD_FILL,
    EMERALD, MUTED, STROKE, TEXT, heatmap_color,
};
use eframe::egui::{self, Color32, Stroke, Vec2};
use egui_extras::{Column, TableBuilder};
use egui_plot::{Line, Plot, PlotPoints, PlotUi};
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceTab {
    Calibration,
    Strategy,
    Telemetry,
    Transmission,
    FlashDiagnostics,
    Switchboard,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BeginnerStep {
    Connect,
    Identify,
    Log,
    Calibrate,
}

pub struct WidebandApp {
    // Hardware & Protocols
    pub devices: Vec<TransportDevice>,
    pub selected_device_idx: usize,
    pub profiles: Vec<EcuProfile>,
    pub selected_profile_idx: usize,
    pub active_device: Option<Box<dyn PassThruDevice>>,
    pub active_device_virtual: bool,
    pub is_connected: bool,

    // Calibration & XDF
    pub xdf: XdfDocument,
    pub selected_table_idx: usize,
    pub search_query: String,
    pub stock_tables: Vec<Vec<Vec<f64>>>,
    pub stock_constants: Vec<f64>,
    /// Second tune decoded with the current XDF (Compare).
    pub compare: Option<crate::compare::CompareFile>,
    /// Cell deltas / edited markers measure against the compare file instead of stock.
    pub diff_vs_compare: bool,
    pub stock_flags: Vec<bool>,
    pub favorites: FavoriteTables,
    pub favorites_path: PathBuf,
    pub favorites_only: bool,
    pub active_cell: (usize, usize),
    /// Other corner of a block selection (None = single cell at `active_cell`).
    pub selection_anchor: Option<(usize, usize)>,
    /// True while a drag-select started in the grid is in progress.
    grid_drag_select: bool,
    pub table_undo: UndoStack,
    /// Typed value being entered for the selection (None = not typing).
    pub cell_entry: Option<String>,
    /// How Ctrl+V combines clipboard cells (paste special).
    pub paste_mode: table_edit::PasteMode,
    /// Log-based VE correction state (panel above the grid).
    pub ve_corr: VeCorrectionUi,
    pub hist: HistogramUi,
    /// Scanner dashboard gauges (dashboard.json).
    pub dashboard: crate::gauges::Dashboard,
    pub dash_editing: bool,
    /// Header row-1 group widths measured last frame (decides whether they share a row).
    hdr_left_w: f32,
    hdr_right_w: f32,
    /// Free-text notes saved inside the .wbtune file.
    pub tune_notes: String,
    /// Calibration tool panel shown above the grid (one at a time).
    cal_tool: Option<CalTool>,
    /// Live operating point on the selected table (separate from the edit selection).
    pub live_trace: live_trace::LiveTrace,
    pub math_amount: f64,
    pub checksum_status: String,
    pub active_rom: Vec<u8>,
    pub rom_origin: i64,
    pub rom_path: Option<PathBuf>,
    /// `active_rom` holds a stock read from the ECU (no file path, but real data).
    pub rom_from_ecu: bool,
    pub xdf_path: Option<PathBuf>,
    pub definition_catalog: DefinitionPackCatalog,
    pub definition_pack_label: String,
    pub plugin_registry: PluginRegistry,

    // Telemetry
    pub telemetry: Arc<Mutex<TelemetryBuffer>>,
    pub trace_rpm: f32,
    pub trace_map: f32,
    pub live_poller_active: bool,
    pub last_poll_time: Instant,
    pub telemetry_playback: TelemetryPlaybackState,
    pub idle_tester_present: IdleTesterPresent,

    // 3D Surface Mesh
    pub mesh_yaw: f32,
    pub mesh_pitch: f32,
    pub mesh_zoom: f32,

    // Flash & Diagnostics
    pub ecu_id: EcuIdentification,
    pub comm_log: Vec<String>,
    pub transfer_progress: f32,
    pub is_flashing: bool,
    pub flash_layout: FlashLayoutMap,
    pub flash_audit: FlashAuditLog,
    pub flash_status_detail: String,
    pub flash_confirm_open: bool,
    pub diag_snapshot: DiagnosticsSnapshot,
    pub diag_selected_dtc: usize,
    pub diag_last_error: String,
    pub connection_last_error: String,
    pub header_vbatt: Option<f32>,

    // Toyota update assistant (Techstream / TIS hand-off)
    pub update_before: Option<UpdateSnapshot>,
    pub update_file_check: Option<UpdateFileCheck>,
    pub update_file_name: String,
    pub update_expected_cal: String,
    pub update_result: Option<UpdateComparison>,
    /// Label of the cable actually connected; the dropdown can change while connected.
    pub connected_device_label: String,

    // TCM (paired with ECM profile)
    pub tcm_profiles: Vec<TcmProfile>,
    pub selected_tcm_profile_idx: usize,
    pub tcm_id: EcuIdentification,
    pub tcm_identify_error: String,
    pub tcm_param_view: TcmParameterView,
    pub tcm_flash_confirm_open: bool,

    // Strategy / patches
    pub strategy_manager_idx: usize,
    pub loaded_patch: Option<crate::patch_recipe::PatchRecipe>,
    pub patch_recipe_path: Option<PathBuf>,
    pub patch_preview: Option<crate::patch_recipe::PatchPreview>,
    pub patch_status: String,

    // UI Workspace
    pub active_tab: WorkspaceTab,
    pub simple_ux: SimpleUxState,
}

impl WidebandApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply_motorsport_theme(&cc.egui_ctx);
        let devices = TransportRegistry::discover();
        let plugin_registry = PluginRegistry::load_default();
        if !plugin_registry.load_note.is_empty() {
            eprintln!("plugins: {}", plugin_registry.load_note);
        }
        let catalog = crate::profile_pack::ProfileCatalog::load_default();
        let profiles = catalog.profiles;
        if !catalog.load_note.is_empty() {
            eprintln!("profiles: {}", catalog.load_note);
        }
        let tcm_catalog = TcmProfileCatalog::load_default();
        let tcm_profiles = tcm_catalog.profiles;
        if !tcm_catalog.load_note.is_empty() {
            eprintln!("tcm profiles: {}", tcm_catalog.load_note);
        }
        let definition_catalog = DefinitionPackCatalog::load_default();
        if !definition_catalog.load_note.is_empty() {
            eprintln!("definition-packs: {}", definition_catalog.load_note);
        }
        let xdf = XdfDocument::sample();
        let tcm_param_view = build_parameter_view(&xdf);

        let stock_tables = xdf.tables.iter().map(|t| t.values.clone()).collect();
        let stock_constants = xdf.constants.iter().map(|c| c.value).collect();
        let stock_flags = xdf.flags.iter().map(|f| f.is_set).collect();
        let favorites_path = FavoriteTables::default_path();
        let favorites = FavoriteTables::load(&favorites_path);
        let mut rom = vec![0xFFu8; 1024 * 1024]; // 1MB mock ROM buffer
        xdf.write_all_tables_to_rom(&mut rom, 0);

        PlatformSecurity::install(&PlatformSecurity::default_config_path());
        let flash_layout = FlashLayoutMap::load(FlashLayoutMap::default_path())
            .unwrap_or_else(|e| {
                eprintln!("flash-layout.json: {}", e);
                FlashLayoutMap::unavailable(e)
            });
        let flash_audit = FlashAuditLog::new(FlashAuditLog::default_path());

        let selected_device_idx = TransportDevice::preferred_index(&devices);
        let mut app = Self {
            devices,
            selected_device_idx,
            profiles,
            selected_profile_idx: 0,
            active_device: None,
            active_device_virtual: false,
            is_connected: false,
            xdf,
            selected_table_idx: 0,
            search_query: String::new(),
            stock_tables,
            stock_constants,
            compare: None,
            diff_vs_compare: false,
            stock_flags,
            favorites,
            favorites_path,
            favorites_only: false,
            active_cell: (0, 0),
            selection_anchor: None,
            grid_drag_select: false,
            table_undo: UndoStack::default(),
            cell_entry: None,
            paste_mode: table_edit::PasteMode::Replace,
            ve_corr: VeCorrectionUi::default(),
            hist: HistogramUi::default(),
            dashboard: {
                let ids: Vec<String> = TelemetryChannelFile::load_default().channels.iter().map(|c| c.id.clone()).collect();
                crate::gauges::Dashboard::load(&crate::gauges::Dashboard::default_path())
                    .unwrap_or_else(|_| crate::gauges::Dashboard::default_layout(&ids))
            },
            dash_editing: false,
            hdr_left_w: 0.0,
            hdr_right_w: 0.0,
            tune_notes: String::new(),
            cal_tool: None,
            live_trace: live_trace::LiveTrace::default(),
            math_amount: 5.0,
            checksum_status: CHECKSUM_VALID.to_string(),
            active_rom: rom,
            rom_origin: 0,
            rom_path: None,
            rom_from_ecu: false,
            xdf_path: None,
            definition_catalog,
            definition_pack_label: String::new(),
            plugin_registry,
            telemetry: Arc::new(Mutex::new(
                TelemetryBuffer::from_definition(&TelemetryChannelFile::load_default(), None),
            )),
            trace_rpm: 2400.0,
            trace_map: 70.0,
            live_poller_active: true,
            last_poll_time: Instant::now(),
            telemetry_playback: TelemetryPlaybackState::default(),
            idle_tester_present: IdleTesterPresent::new(),
            mesh_yaw: 0.785,  // 45 degrees
            mesh_pitch: 0.523, // 30 degrees
            mesh_zoom: 1.0,
            ecu_id: EcuIdentification::default(),
            comm_log: vec!["Wideband Motorsport Studio 2.4 (Rust Native) Initialized.".to_string()],
            transfer_progress: 0.0,
            is_flashing: false,
            flash_layout,
            flash_audit,
            flash_status_detail: String::new(),
            flash_confirm_open: false,
            diag_snapshot: DiagnosticsSnapshot::default(),
            diag_selected_dtc: 0,
            diag_last_error: String::new(),
            connection_last_error: String::new(),
            header_vbatt: None,
            update_before: UpdateSnapshot::load(&UpdateSnapshot::default_before_path()).ok(),
            update_file_check: None,
            update_file_name: String::new(),
            update_expected_cal: String::new(),
            update_result: None,
            connected_device_label: String::new(),
            tcm_profiles,
            selected_tcm_profile_idx: 0,
            tcm_id: EcuIdentification::default(),
            tcm_identify_error: String::new(),
            tcm_param_view,
            tcm_flash_confirm_open: false,
            strategy_manager_idx: 0,
            loaded_patch: None,
            patch_recipe_path: None,
            patch_preview: None,
            patch_status: String::new(),
            active_tab: WorkspaceTab::Calibration,
            simple_ux: SimpleUxState::load(),
        };

        if let Some(idx) = app
            .tcm_profiles
            .iter()
            .position(|p| p.paired_ecm_profile_id == app.profiles[app.selected_profile_idx].id)
        {
            app.selected_tcm_profile_idx = idx;
        }

        let sec_note = PlatformSecurity::load_note();
        if !sec_note.is_empty() {
            app.log(&format!("Security config: {}", sec_note));
        }
        if let Some(err) = app.flash_layout.load_error().map(str::to_string) {
            app.log(&format!(
                "Flash layout unavailable ({}). Read Stock ROM and flash are disabled until flash-layout.json sits beside wideband.exe.",
                err
            ));
        }

        app.select_profile_id("tacoma-can");
        app.sync_tcm_profile_to_ecm();
        app.try_load_definition_pack_for_profile();
        if !app.simple_ux.wizard_open {
            app.connect_cable();
        }
        app
    }

    fn try_load_definition_pack_for_profile(&mut self) {
        let profile_id = self.profiles[self.selected_profile_idx].id.clone();
        self.definition_pack_label.clear();
        let pack_meta = self
            .definition_catalog
            .best_for_profile(&profile_id)
            .map(|pack| (pack.title.clone(), pack.id.clone()));
        let xdf_path = self.definition_catalog.primary_xdf_path(&profile_id);
        if let Some((title, id)) = pack_meta {
            self.definition_pack_label = format!("Pack: {} ({})", title, id);
            if let Some(path) = xdf_path {
                self.load_xdf_from_path(path);
                self.log(&format!(
                    "Definition pack '{}' loaded primary XDF for {}",
                    id,
                    profile_id
                ));
            }
        }
    }

    fn run_profile_plugin_hooks(&self, profile: &EcuProfile, image: &[u8], origin: i64) -> Result<(), String> {
        if let Some(id) = &profile.plugins.checksum {
            let msg = PluginRegistry::invoke_checksum_hook(id, &profile.id, image, origin)?;
            if !msg.is_empty() {
                return Ok(());
            }
        }
        Ok(())
    }

    fn set_selected_profile_idx(&mut self, idx: usize) {
        if idx == self.selected_profile_idx {
            return;
        }
        self.selected_profile_idx = idx;
        self.sync_tcm_profile_to_ecm();
        self.try_load_definition_pack_for_profile();
    }

    fn commit_table_edits(&mut self) {
        let idx = self.selected_table_idx;
        if self.xdf.write_table_to_rom(&mut self.active_rom, self.rom_origin, idx) {
            // Show what the ROM actually holds after equation inverse + integer quantisation.
            self.xdf.reload_table_from_rom(&self.active_rom, self.rom_origin, idx);
        }
        self.refresh_checksum_badge();
    }

    fn refresh_stock_baseline(&mut self) {
        self.stock_tables = self.xdf.tables.iter().map(|t| t.values.clone()).collect();
        self.stock_constants = self.xdf.constants.iter().map(|c| c.value).collect();
        self.stock_flags = self.xdf.flags.iter().map(|f| f.is_set).collect();
    }

    /// True while `active_rom` is still the startup 0xFF buffer (no file opened, no stock read).
    pub(crate) fn rom_is_placeholder(&self) -> bool {
        self.rom_path.is_none() && !self.rom_from_ecu
    }

    pub(crate) fn refresh_checksum_badge(&mut self) {
        let prof = &self.profiles[self.selected_profile_idx];
        self.checksum_status =
            PlatformSecurity::checksum_badge(&prof.checksum_profile_id, &self.active_rom, self.rom_origin);
    }

    fn persist_favorites(&mut self) {
        if let Err(e) = self.favorites.save(&self.favorites_path) {
            self.log(&format!("Favorites save failed: {}", e));
        }
    }

    fn toggle_table_favorite(&mut self, table_id: &str) {
        self.favorites.toggle(table_id);
        self.persist_favorites();
    }

    fn load_rom_from_path(&mut self, path: PathBuf) {
        let label = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(e) => return self.log(&format!("Open ROM failed: {}", e)),
        };
        let load_result: Result<String, String> = match RomImage::from_bytes(&ext, &bytes) {
            Ok(img) => {
                self.rom_origin = img.origin as i64;
                img.load_into_buffer(&mut self.active_rom, ext != "cuw");
                let note = if img.note.is_empty() {
                    String::new()
                } else {
                    format!(" — {}", img.note.lines().next().unwrap_or(""))
                };
                Ok(format!(
                    "Loaded {} {} ({} bytes @ 0x{:X}{})",
                    if ext == "cuw" { "CUW" } else { "ROM" },
                    label,
                    img.bytes.len(),
                    img.origin,
                    note
                ))
            }
            Err(e) => Err(e),
        };

        match load_result {
            Ok(msg) => {
                self.rom_path = Some(path);
                self.xdf.bind_from_rom(&self.active_rom, self.rom_origin);
                self.refresh_stock_baseline();
                self.table_undo.clear();
                self.refresh_checksum_badge();
                self.log(&msg);
            }
            Err(e) => self.log(&format!("Open ROM failed: {}", e)),
        }
    }

    fn load_xdf_from_path(&mut self, path: PathBuf) {
        match fs::read_to_string(&path) {
            Ok(xml) => match XdfDocument::parse(&xml) {
                Ok(doc) => {
                    self.xdf = doc;
                    self.tcm_param_view = build_parameter_view(&self.xdf);
                    self.selected_table_idx = 0;
                    self.active_cell = (0, 0);
                    self.selection_anchor = None;
                    self.table_undo.clear();
                    self.cell_entry = None;
                    self.compare = None;
                    self.diff_vs_compare = false;
                    self.live_trace.clear();
                    let label = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                    self.xdf_path = Some(path);
                    self.xdf.bind_from_rom(&self.active_rom, self.rom_origin);
                    self.refresh_stock_baseline();
                    self.refresh_checksum_badge();
                    self.log(&format!("Loaded XDF {} ({} tables)", label, self.xdf.tables.len()));
                    for w in self.xdf.warnings.clone() {
                        self.log(&format!("XDF: {}", w));
                    }
                }
                Err(e) => self.log(&format!("XDF parse failed: {}", e)),
            },
            Err(e) => self.log(&format!("Open XDF failed: {}", e)),
        }
    }

    fn save_rom_to_path(&mut self, path: PathBuf) {
        let prof = &self.profiles[self.selected_profile_idx];
        self.xdf.write_all_tables_to_rom(&mut self.active_rom, self.rom_origin);
        for flag in &self.xdf.flags {
            self.xdf.write_flag_to_rom(&mut self.active_rom, self.rom_origin, flag);
        }

        let mut image = self.active_rom.clone();
        match PlatformSecurity::stamp_image(&prof.checksum_profile_id, &mut image, self.rom_origin) {
            Ok(status) => {
                if let Err(e) = self.run_profile_plugin_hooks(prof, &image, self.rom_origin) {
                    self.log(&format!("Plugin checksum hook failed (fail-closed): {}", e));
                    return;
                }
                self.checksum_status = status;
                let nbytes = image.len();
                match fs::write(&path, &image) {
                    Ok(()) => {
                        self.active_rom = image;
                        self.rom_path = Some(path.clone());
                        self.log(&format!(
                            "Saved ROM {} ({} bytes, {})",
                            path.file_name().unwrap_or_default().to_string_lossy(),
                            nbytes,
                            self.checksum_status
                        ));
                    }
                    Err(e) => self.log(&format!("Save ROM failed: {}", e)),
                }
            }
            Err(e) => self.log(&format!("Save ROM blocked: {}", e)),
        }
    }

    pub fn log(&mut self, text: &str) {
        let time = chrono::Local::now().format("%H:%M:%S%.3f");
        self.comm_log.push(format!("[{}] {}", time, text));
        if self.comm_log.len() > 1000 {
            self.comm_log.remove(0);
        }
    }

    pub fn rescan_devices(&mut self) {
        if self.is_connected {
            self.log("Rescan skipped while connected — disconnect first.");
            return;
        }
        self.devices = TransportRegistry::discover();
        if self.selected_device_idx >= self.devices.len() {
            self.selected_device_idx = 0;
        }
        self.log(&format!(
            "Device scan: {} transport(s) (USB J2534, Bluetooth, Ethernet, virtual bench).",
            self.devices.len()
        ));
    }

    fn friendly_connect_open_error(raw: &str) -> String {
        let lower = raw.to_lowercase();
        if lower.contains("j2534host.exe not found") || lower.contains("j2534host.exe is missing") {
            return crate::j2534::missing_j2534_host_message();
        }
        if lower.contains("could not load") || lower.contains("loadlibrary") {
            return format!(
                "{} — If this is a clone cable DLL, run device_check.ps1 -DllPath \"<FunctionLibrary>\" and scripts\\Friday-CablePrep.ps1 before connect.",
                raw
            );
        }
        raw.to_string()
    }

    pub fn connect_cable(&mut self) {
        self.connection_last_error.clear();
        self.header_vbatt = None;
        if self.selected_device_idx >= self.devices.len() {
            self.connection_last_error = "No cable device selected.".to_string();
            return;
        }
        let dev_row = self.devices[self.selected_device_idx].clone();
        let is_virtual = dev_row.is_virtual_bench();
        let dev_name = dev_row.list_label();
        let transport = dev_row.transport_hint();

        if is_virtual {
            self.log(
                "Virtual ECU selected: responses are synthetic bench data, not your Tacoma on a real Pass-Thru cable.",
            );
        }

        if !dev_row.is_connectable() {
            self.connection_last_error = match &dev_row.availability {
                TransportAvailability::Offline => format!(
                    "{} is offline — check power, pairing, or Wi-Fi.",
                    dev_row.display_name
                ),
                TransportAvailability::MissingDll => {
                    "PassThru DLL not found for this registry entry.".to_string()
                }
                TransportAvailability::NotConfigured => {
                    "Wireless adapter not configured — see docs/wireless-transport-adapters.md."
                        .to_string()
                }
                TransportAvailability::Error(msg) => msg.clone(),
                TransportAvailability::Ready => "Device is not available.".to_string(),
            };
            self.log(&format!(
                "{}. Install drivers or set WIDEBAND_BT_SERIAL / WIDEBAND_ETH_GATEWAYS, then Rescan.",
                self.connection_last_error
            ));
            return;
        }

        if !is_virtual
            && dev_row.kind == crate::transport::TransportKind::UsbJ2534
            && crate::j2534::requires_j2534_host_bridge()
            && !crate::j2534::j2534_host_exe_present()
        {
            let msg = crate::j2534::missing_j2534_host_message();
            self.connection_last_error = msg.clone();
            self.log(&msg);
            return;
        }

        self.log(&format!("Opening {} via {}", dev_name, transport));

        let mut dev = match dev_row.open_pass_thru() {
            Ok(d) => d,
            Err(e) => {
                self.connection_last_error = Self::friendly_connect_open_error(&e);
                self.devices[self.selected_device_idx].availability =
                    TransportAvailability::Error(self.connection_last_error.clone());
                self.log(&format!(
                    "Failed to open {}: {} (check USB, driver, pairing, or gateway)",
                    dev_name,
                    self.connection_last_error
                ));
                return;
            }
        };
        match dev.open() {
            Err(e) => {
                self.connection_last_error = Self::friendly_connect_open_error(&e);
                self.devices[self.selected_device_idx].availability =
                    TransportAvailability::Error(self.connection_last_error.clone());
                self.log(&format!(
                    "Failed to open {}: {} (check USB, driver, and VX Manager / MVCI license if applicable)",
                    dev_name,
                    self.connection_last_error
                ));
                return;
            }
            Ok(()) => {}
        }

        if is_virtual {
            PlatformSecurity::register_bench_keys_for_virtual();
            self.log("Virtual bench: explicit zero-mask security keys registered for loopback only.");
        } else {
            PlatformSecurity::install(&PlatformSecurity::default_config_path());
            let sec_note = PlatformSecurity::load_note();
            if !sec_note.is_empty() {
                self.log(&format!("Security config: {}", sec_note));
            }
        }

        let prof = &self.profiles[self.selected_profile_idx];
        match connect_for_profile(dev.as_mut(), prof) {
            Ok(ch) => self.log(&format!(
                "PassThru channel {} open — profile {} ({} @ {} bit/s, RX filter 0x{:X})",
                ch,
                prof.id,
                prof.name,
                prof.bitrate,
                prof.response_id
            )),
            Err(e) => {
                self.connection_last_error = e.clone();
                self.log(&format!("Channel setup failed: {}", e));
                let _ = dev.close();
                return;
            }
        }

        let ver = dev.firmware_version();
        self.log(&format!("Connected to {}: {}", dev_name, ver));
        let ch = dev.channel_id();
        self.header_vbatt = read_pass_thru_vbatt(dev.as_mut(), ch);
        self.active_device_virtual = is_virtual;
        self.active_device = Some(dev);
        self.is_connected = true;
        self.connected_device_label = dev_name.clone();
        self.connection_last_error.clear();
        if is_virtual {
            self.idle_tester_present.disarm();
        } else {
            self.idle_tester_present.arm_for_real_hardware();
            self.log(
                "Idle session hold: TesterPresent (0x3E) every 2 s on this profile (real Pass-Thru only).",
            );
        }
    }

    pub fn disconnect_cable(&mut self) {
        self.idle_tester_present.disarm();
        self.live_trace.clear();
        if let Some(mut dev) = self.active_device.take() {
            let _ = dev.close();
            self.is_connected = false;
            self.header_vbatt = None;
            PlatformSecurity::install(&PlatformSecurity::default_config_path());
            self.log("Cable disconnected.");
        }
    }

    pub fn poll_idle_tester_present(&mut self) {
        if !self.is_connected || self.active_device_virtual || self.is_flashing {
            return;
        }
        let prof = self.profiles[self.selected_profile_idx].clone();
        if let Some(dev) = &mut self.active_device {
            match self.idle_tester_present.tick(
                dev.as_mut(),
                &prof,
                self.is_connected,
                self.active_device_virtual,
                self.is_flashing,
            ) {
                IdleTesterOutcome::Skipped | IdleTesterOutcome::Sent => {}
                IdleTesterOutcome::Failed => {
                    self.log(
                        "TesterPresent was not answered; idle keepalive stopped (channel stays open). Reconnect to retry.",
                    );
                }
            }
        }
    }

    pub(crate) fn cable_is_virtual(&self) -> bool {
        if self.is_connected {
            self.active_device_virtual
        } else {
            self.devices
                .get(self.selected_device_idx)
                .map(|d| d.is_virtual_bench())
                .unwrap_or(true)
        }
    }

    fn render_security_status_panel(&mut self, ui: &mut egui::Ui) {
        let prof = &self.profiles[self.selected_profile_idx];
        let virtual_ecu = self.cable_is_virtual();
        let report = PlatformSecurity::security_report(&prof.id, virtual_ecu);
        let mode_color = match report.mode {
            SecurityMode::Production => EMERALD,
            SecurityMode::Bench => Color32::from_rgb(0xF5, 0x88, 0x36),
            SecurityMode::FailClosed => Color32::from_rgb(255, 23, 68),
        };

        egui::Frame::none()
            .fill(CARD_FILL)
            .stroke(Stroke::new(1.0, STROKE))
            .rounding(egui::Rounding::same(6.0))
            .inner_margin(egui::Margin::symmetric(12.0, 10.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading(
                        egui::RichText::new("Security config")
                            .size(14.0)
                            .color(ACCENT_CYAN),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(security_mode_label(report.mode))
                                .strong()
                                .color(mode_color),
                        );
                    });
                });
                ui.label(
                    egui::RichText::new(format!("File: {}", report.config_path))
                        .small()
                        .color(MUTED),
                );
                if !report.load_note.is_empty() {
                    ui.label(
                        egui::RichText::new(&report.load_note)
                            .small()
                            .color(Color32::from_rgb(255, 120, 120)),
                    );
                }
                ui.add_space(4.0);
                ui.label(egui::RichText::new("Profiles (seed-key 0x27):").small().strong());
                for line in &report.profiles {
                    let key_color = match line.key_registration {
                        KeyRegistration::Json => EMERALD,
                        KeyRegistration::Bench => Color32::from_rgb(0xF5, 0x88, 0x36),
                        KeyRegistration::None => MUTED,
                    };
                    let json_flag = if line.configured_in_json { "configured: true" } else { "configured: false" };
                    ui.label(
                        egui::RichText::new(format!(
                            "  {} [{}] — {} — key {}",
                            line.profile_id,
                            line.algorithm,
                            json_flag,
                            key_registration_label(line.key_registration),
                        ))
                        .small()
                        .color(key_color),
                    );
                }
                ui.add_space(4.0);
                ui.label(egui::RichText::new("Checksum ranges:").small().strong());
                if report.checksums.is_empty() {
                    ui.label(egui::RichText::new("  (none declared in JSON)").small().color(MUTED));
                } else {
                    for line in &report.checksums {
                        let status = if line.active {
                            "active"
                        } else if line.configured_in_json {
                            "configured but not loaded"
                        } else {
                            "placeholder"
                        };
                        ui.label(
                            egui::RichText::new(format!(
                                "  {} profile={} {} — {}",
                                line.id,
                                if line.profile.is_empty() { "any" } else { &line.profile },
                                line.algorithm,
                                status,
                            ))
                            .small()
                            .color(if line.active { EMERALD } else { MUTED }),
                        );
                    }
                }
                if !report.flash_blockers.is_empty() {
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new("Flash blocked until resolved:")
                            .small()
                            .strong()
                            .color(Color32::from_rgb(255, 23, 68)),
                    );
                    for reason in &report.flash_blockers {
                        ui.label(egui::RichText::new(format!("  • {}", reason)).small());
                    }
                }
            });
    }

    pub fn poll_telemetry(&mut self) {
        if !self.is_connected || !self.live_poller_active || self.is_flashing {
            return;
        }
        // The J2534Host bridge closes itself when the host/DLL hangs or exits; reflect that in
        // the UI instead of polling a dead session forever.
        if self.active_device.as_ref().map_or(false, |d| !d.is_connected()) {
            self.log("Cable session lost (J2534Host closed or not responding). Reconnect to continue.");
            self.disconnect_cable();
            return;
        }

        let tick = {
            let buf = self.telemetry.lock().unwrap();
            buf.scheduler.tick_interval()
        };
        if self.last_poll_time.elapsed() < tick {
            return;
        }
        self.last_poll_time = Instant::now();
        let now = Instant::now();

        if let Some(dev) = &mut self.active_device {
            let ch = dev.channel_id();
            if let Some(v) = read_pass_thru_vbatt(dev.as_mut(), ch) {
                self.header_vbatt = Some(v);
            }
            let prof = &self.profiles[self.selected_profile_idx];
            let poll_list: Vec<(usize, Vec<u8>, String)> = {
                let mut buf = self.telemetry.lock().unwrap();
                let plan = buf.scheduler.plan(&buf.channels, now);
                buf.scheduler.apply_budget_skips(&plan.budget_skipped, now);
                let poll_list = plan
                    .poll_indices
                    .into_iter()
                    .map(|idx| {
                        let ch = &buf.channels[idx];
                        (
                            idx,
                            ch.build_request_bytes(),
                            ch.decode.clone(),
                        )
                    })
                    .collect();
                poll_list
            };

            let channel = dev.channel_id();
            for (idx, req, decode_id) in poll_list {
                let mut ok = false;
                if let Ok(()) =
                    crate::protocols::IsoTpLink::send(dev.as_mut(), channel, prof.request_id, &req)
                {
                    if let Ok(resp) = crate::protocols::IsoTpLink::receive(dev.as_mut(), channel, 10) {
                        if let Some(value) = decode_channel_value(&decode_id, &resp) {
                            let mut buf = self.telemetry.lock().unwrap();
                            buf.set_channel_value(idx, value, now);
                            buf.scheduler.mark_polled(idx, now);
                            ok = true;
                        }
                    }
                }
                if !ok {
                    let mut buf = self.telemetry.lock().unwrap();
                    buf.bus_fail_count += 1;
                }
            }
            let live_pos = {
                let mut buf = self.telemetry.lock().unwrap();
                self.trace_rpm = buf.value("rpm");
                self.trace_map = buf.value("map");
                buf.push_frame();
                // Live operating point on the selected table. Deliberately NOT written to
                // `active_cell`: that is the user's edit selection, and math ops write to it.
                let table = self.xdf.tables.get(self.selected_table_idx);
                self.live_trace.out_of_range = table
                    .map(|t| live_trace::out_of_range(t, |id| buf.value(id)))
                    .unwrap_or_default();
                table.and_then(|t| live_trace::table_position(t, |id| buf.value(id)))
            };
            match live_pos {
                Some(pos) => self.live_trace.push(pos, now),
                None => self.live_trace.clear(),
            }
        }
    }

    /// Selection rectangle in the grid (anchor..cursor); the cursor is `active_cell`.
    pub fn selection(&self) -> CellSelection {
        CellSelection {
            anchor: self.selection_anchor.unwrap_or(self.active_cell),
            cursor: self.active_cell,
        }
    }

    /// Apply `edit` to the selected table's values with an undo snapshot, then commit to ROM.
    fn edit_selected_table(&mut self, label: &str, edit: impl FnOnce(&mut [Vec<f64>], &CellSelection)) {
        if self.xdf.tables.is_empty() {
            return;
        }
        let idx = self.selected_table_idx;
        if let Err(e) = self.xdf.tables[idx].format.check() {
            // Never edit a table we can't encode back faithfully.
            self.log(&format!("\"{}\" is read-only: {}", self.xdf.tables[idx].title, e));
            return;
        }
        let sel = self.selection();
        let before = self.xdf.tables[idx].values.clone();
        edit(&mut self.xdf.tables[idx].values, &sel);
        if self.xdf.tables[idx].values == before {
            return;
        }
        self.table_undo.record(idx, before, label);
        self.commit_table_edits();
    }

    pub fn run_table_math(&mut self, op: MathOp) {
        let label = match op {
            MathOp::Add(a) => format!("+{}", a),
            MathOp::Sub(a) => format!("-{}", a),
            MathOp::Percent(p) => format!("{:+}%", p),
            MathOp::Set(a) => format!("= {}", a),
        };
        self.edit_selected_table(&label, |v, sel| table_edit::apply_math(v, sel, op));
    }

    pub fn undo_table_edit(&mut self) {
        let tables = &self.xdf.tables;
        if let Some((idx, values, label)) = self.table_undo.undo(|i| tables[i].values.clone()) {
            self.restore_table_values(idx, values);
            self.log(&format!("Undo: {}", label));
        }
    }

    pub fn redo_table_edit(&mut self) {
        let tables = &self.xdf.tables;
        if let Some((idx, values, label)) = self.table_undo.redo(|i| tables[i].values.clone()) {
            self.restore_table_values(idx, values);
            self.log(&format!("Redo: {}", label));
        }
    }

    fn restore_table_values(&mut self, idx: usize, values: Vec<Vec<f64>>) {
        if idx >= self.xdf.tables.len() {
            return;
        }
        // Show the table being changed so an undo never edits something off-screen.
        self.selected_table_idx = idx;
        self.xdf.tables[idx].values = values;
        self.commit_table_edits();
    }

    fn render_studio_header(&mut self, ctx: &egui::Context, tier: theme::StudioHeaderTier) {
        use theme::StudioHeaderTier;
        let screen_w = ctx.screen_rect().width();
        let compact_badge = tier != StudioHeaderTier::Wide || screen_w < 1500.0;
        let profile_combo_w = match tier {
            StudioHeaderTier::Compact => 130.0,
            StudioHeaderTier::Medium => 170.0,
            StudioHeaderTier::Wide => 200.0,
        };
        let device_combo_w = match tier {
            StudioHeaderTier::Compact => 110.0,
            StudioHeaderTier::Medium => 140.0,
            StudioHeaderTier::Wide => 180.0,
        };
        // Sized to its content (no exact_height): the old fixed 76/112 px panel clipped wrapped
        // rows, which showed as a black bar and could push the whole workspace off-screen.
        egui::TopBottomPanel::top("header_bar")
            .frame(theme::header_frame())
            .show(ctx, |ui| {
                // Header text never wraps (a squeezed chip used to stack one letter per line).
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);

                // Row 1: identity + status on the left, connection on the right. Whether the
                // right group fits beside the left one is decided from last frame's measured
                // widths, so the two groups can never draw on top of each other.
                let fits = self.hdr_left_w > 0.0
                    && ui.available_width() >= self.hdr_left_w + self.hdr_right_w + 24.0;
                let row_h = 34.0;
                ui.horizontal(|ui| {
                    // Left group measured on its own so the fit test is stable frame to frame.
                    let left = ui
                        .horizontal(|ui| {
                            paint_studio_badge(ui, compact_badge);
                            ui.add_space(8.0);
                            self.paint_profile_selector(ui, profile_combo_w);
                            ui.add_space(4.0);
                            self.paint_header_status_chips(ui, screen_w < 1400.0);
                        })
                        .response;
                    self.hdr_left_w = left.rect.width();
                    if fits {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let r = ui.horizontal(|ui| self.paint_connect_controls(ui, device_combo_w)).response;
                            self.hdr_right_w = r.rect.width();
                        });
                    }
                });
                if !fits {
                    // Own row, right-aligned, fixed height (a bare right-to-left layout here
                    // would claim all remaining height and push the tabs off-screen).
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), row_h),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            let r = ui.horizontal(|ui| self.paint_connect_controls(ui, device_combo_w)).response;
                            self.hdr_right_w = r.rect.width();
                        },
                    );
                }
                ui.add_space(4.0);

                // Row 2: workspace tabs, then experience / practice controls; wraps as a unit
                // so every tab stays visible at any width.
                ui.horizontal_wrapped(|ui| {
                    if !self.simple_ux.is_beginner() {
                        self.paint_workspace_tabs(ui, tier != StudioHeaderTier::Wide);
                        ui.separator();
                    }
                    self.render_skill_and_practice_bar(ui);
                });
            });
    }

    fn sync_tcm_profile_to_ecm(&mut self) {
        let ecm_id = self.profiles[self.selected_profile_idx].id.clone();
        if let Some(idx) = self
            .tcm_profiles
            .iter()
            .position(|p| p.paired_ecm_profile_id == ecm_id)
        {
            self.selected_tcm_profile_idx = idx;
        }
    }

    fn selected_tcm_profile(&self) -> &TcmProfile {
        &self.tcm_profiles[self.selected_tcm_profile_idx.min(self.tcm_profiles.len().saturating_sub(1))]
    }

    pub(crate) fn paint_profile_selector(&mut self, ui: &mut egui::Ui, width: f32) {
        ui.label(egui::RichText::new("Vehicle").size(11.0).color(MUTED));
        let mut picked: Option<usize> = None;
        egui::ComboBox::from_id_salt("profile_select")
            .width(width)
            .selected_text(&self.profiles[self.selected_profile_idx].name)
            .show_ui(ui, |ui| {
                for (idx, prof) in self.profiles.iter().enumerate() {
                    if ui
                        .selectable_label(self.selected_profile_idx == idx, &prof.name)
                        .clicked()
                    {
                        picked = Some(idx);
                    }
                }
            });
        if let Some(idx) = picked {
            self.set_selected_profile_idx(idx);
        }
    }

    pub(crate) fn paint_connect_controls(&mut self, ui: &mut egui::Ui, device_combo_w: f32) {
        if self.is_connected {
            if ui
                .button(egui::RichText::new("Disconnect").size(11.0).color(MUTED))
                .clicked()
            {
                self.disconnect_cable();
            }
        } else if ui
            .button(
                egui::RichText::new("Connect")
                    .size(11.0)
                    .strong()
                    .color(ACCENT_CYAN),
            )
            .clicked()
        {
            self.connect_cable();
        }
        ui.add_space(6.0);
        egui::ComboBox::from_id_salt("device_select")
            .width(device_combo_w)
            .selected_text(
                egui::RichText::new(self.devices[self.selected_device_idx].list_label())
                    .size(11.0)
                    .color(MUTED),
            )
            .show_ui(ui, |ui| {
                for (idx, dev) in self.devices.iter().enumerate() {
                    ui.selectable_value(&mut self.selected_device_idx, idx, dev.list_label());
                }
            });
        if ui
            .button(egui::RichText::new("Rescan").size(10.0).color(MUTED))
            .on_hover_text("Refresh USB J2534 registry and wireless env entries")
            .clicked()
        {
            self.rescan_devices();
        }
    }

    fn paint_workspace_tabs(&mut self, ui: &mut egui::Ui, compact: bool) {
        let tabs = [
            (WorkspaceTab::Switchboard, "Switchboard"),
            (WorkspaceTab::FlashDiagnostics, "Flash & Diag"),
            (WorkspaceTab::Transmission, "Transmission"),
            (WorkspaceTab::Strategy, "Strategy"),
            (WorkspaceTab::Telemetry, "Telemetry"),
            (WorkspaceTab::Calibration, "Calibration"),
        ];
        for (tab, title) in tabs {
            if studio_tab_button(ui, title, self.active_tab == tab, compact).clicked() {
                self.active_tab = tab;
            }
        }
    }

    fn paint_live_telemetry_card(&self, ui: &mut egui::Ui) {
        let live = self.is_connected && self.live_poller_active && !self.is_flashing;
        let table = self.xdf.tables.get(self.selected_table_idx);
        let live_pos = self.live_trace.current();
        egui::Frame::none()
            .fill(CARD_FILL)
            .stroke(Stroke::new(1.0, STROKE))
            .rounding(egui::Rounding::same(4.0))
            .inner_margin(egui::Margin::symmetric(10.0, 8.0))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Live operating point")
                            .size(11.0)
                            .strong()
                            .color(MUTED),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (txt, col) = if live { ("\u{25CF} LIVE", EMERALD) } else { ("IDLE", MUTED) };
                        ui.label(egui::RichText::new(txt).size(11.0).strong().color(col));
                    });
                });
                ui.add_space(4.0);
                let stat = |ui: &mut egui::Ui, name: &str, value: String, color: Color32| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(name).size(10.0).color(MUTED));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(egui::RichText::new(value).size(13.0).monospace().strong().color(color));
                        });
                    });
                };
                let dim = |v: String| if live { v } else { "--".to_string() };
                stat(ui, "Engine RPM", dim(format!("{:.0}", self.trace_rpm)), ACCENT_CYAN);
                stat(ui, "MAP", dim(format!("{:.1} kPa", self.trace_map)), ACCENT_CYAN);
                let live_txt = match (live_pos, table) {
                    (Some((r, c)), Some(t)) => {
                        let (nr, nc) = live_trace::nearest_cell((r, c));
                        let v = t.values.get(nr).and_then(|row| row.get(nc)).copied().unwrap_or(0.0);
                        format!("R{:.1} C{:.1} = {:.1}", r, c, v)
                    }
                    _ if live => "axes not logged".to_string(),
                    _ => "--".to_string(),
                };
                stat(ui, "Live cell", live_txt, EMERALD);
                if live && !self.live_trace.out_of_range.is_empty() {
                    // The ECU clamps to the edge cell: it's doing work it may not be tuned for.
                    ui.label(
                        egui::RichText::new(format!(
                            "\u{26A0} Off table: {} (edge cell used)",
                            self.live_trace.out_of_range.join(", ")
                        ))
                        .size(10.5)
                        .strong()
                        .color(Color32::from_rgb(255, 152, 0)),
                    );
                }
                let (sr, sc) = self.active_cell;
                let sel_val = table
                    .and_then(|t| t.values.get(sr).and_then(|row| row.get(sc)).copied())
                    .map(|v| format!(" = {:.1}", v))
                    .unwrap_or_default();
                stat(ui, "Selected (edit)", format!("R{} C{}{}", sr, sc, sel_val), ACCENT_CYAN);
            });
    }

    fn paint_header_status_chips(&self, ui: &mut egui::Ui, narrow: bool) {
        let (status_text, status_color) = if self.is_connected {
            ("ONLINE", EMERALD)
        } else {
            ("OFFLINE", Color32::from_rgb(0xF8, 0x51, 0x49))
        };
        egui::Frame::none()
            .fill(status_color.gamma_multiply(0.12))
            .stroke(Stroke::new(1.0, status_color))
            .rounding(egui::Rounding::same(4.0))
            .inner_margin(egui::Margin::symmetric(8.0, 4.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("●").size(9.0).color(status_color));
                    ui.label(
                        egui::RichText::new(status_text)
                            .size(11.0)
                            .strong()
                            .color(status_color),
                    );
                    if !self.is_connected && !self.connection_last_error.is_empty() {
                        // Long errors are clipped in the header; the full text is on hover.
                        let max = if narrow { 28 } else { 60 };
                        let err = &self.connection_last_error;
                        let short = if err.chars().count() > max {
                            format!("— {}\u{2026}", err.chars().take(max).collect::<String>())
                        } else {
                            format!("— {}", err)
                        };
                        ui.label(egui::RichText::new(short).size(10.0).color(Color32::from_rgb(0xF8, 0x51, 0x49)))
                            .on_hover_text(err.as_str());
                    }
                });
            });

        if self.cable_is_virtual() {
            ui.add_space(6.0);
            egui::Frame::none()
                .fill(Color32::from_rgb(255, 193, 7).gamma_multiply(0.12))
                .stroke(Stroke::new(1.0, Color32::from_rgb(255, 193, 7)))
                .rounding(egui::Rounding::same(4.0))
                .inner_margin(egui::Margin::symmetric(8.0, 4.0))
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(if narrow { "Virtual ECU" } else { "Virtual ECU — not a real OBD cable" })
                            .size(10.0)
                            .strong()
                            .color(Color32::from_rgb(255, 193, 7)),
                    )
                    .on_hover_text("Bench loopback simulator — not a real OBD cable");
                });
        }

        ui.add_space(6.0);

        let (batt_label, has_batt) = if !self.is_connected {
            ("—".to_string(), false)
        } else {
            let tel_v = self.telemetry.lock().unwrap().value("vbatt");
            if tel_v > 1.0 {
                (format!("{:.1}V", tel_v), true)
            } else if let Some(v) = self.header_vbatt {
                (format!("{:.1}V", v), true)
            } else {
                ("—".to_string(), false)
            }
        };
        let vbatt_color = if has_batt { ACCENT_CYAN } else { MUTED };
        egui::Frame::none()
            .fill(theme::WIDGET_BG)
            .stroke(Stroke::new(1.0, STROKE))
            .rounding(egui::Rounding::same(4.0))
            .inner_margin(egui::Margin::symmetric(8.0, 4.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("VBATT").size(10.0).color(MUTED));
                    ui.label(
                        egui::RichText::new(batt_label)
                            .size(12.0)
                            .strong()
                            .monospace()
                            .color(vbatt_color),
                    );
                });
            });
    }
}

impl eframe::App for WidebandApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_telemetry();
        self.poll_idle_tester_present();
        self.tick_virtual_ladder();
        let dt = ctx.input(|i| i.stable_dt) as f64;
        self.telemetry_playback.tick_playback(dt);
        ctx.request_repaint();

        theme::apply_motorsport_theme(ctx);

        let tier = theme::studio_header_tier(ctx);
        self.render_studio_header(ctx, tier);

        if !self.simple_ux.wizard_open {
            if self.simple_ux.is_beginner() {
                self.render_beginner_workspace(ctx);
            } else {
                match self.active_tab {
                    WorkspaceTab::Calibration => self.render_calibration_tab(ctx, false),
                    WorkspaceTab::Strategy => self.render_strategy_tab(ctx),
                    WorkspaceTab::Telemetry => self.render_telemetry_tab(ctx, false),
                    WorkspaceTab::Transmission => self.render_transmission_tab(ctx),
                    WorkspaceTab::FlashDiagnostics => self.render_flash_tab(ctx),
                    WorkspaceTab::Switchboard => self.render_switchboard_tab(ctx),
                }
            }
        }

        if self.simple_ux.wizard_open {
            egui::CentralPanel::default().frame(theme::panel_frame()).show(ctx, |_| {});
        }
        self.render_ux_overlays(ctx);
    }
}

// ---------------------------------------------------------------------------
// Tab 1: Calibration (Parameter Tree + 2D/3D Heatmap Grid + 3D Surface Mesh)
// ---------------------------------------------------------------------------

impl WidebandApp {
    pub(crate) fn render_calibration_tab(&mut self, ctx: &egui::Context, beginner_mode: bool) {
        let (panel_default, panel_min, panel_max) = theme::side_panel_widths(ctx);
        // Left Parameter Tree
        egui::SidePanel::left("param_tree")
            .resizable(true)
            .default_width(panel_default)
            .width_range(panel_min..=panel_max)
            .frame(theme::panel_frame())
            .show(ctx, |ui| {
                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.add_space(2.0);
                        egui::Frame::none()
                            .fill(theme::BG_FILL)
                            .stroke(Stroke::new(1.0, STROKE))
                            .rounding(egui::Rounding::same(4.0))
                            .inner_margin(egui::Margin::symmetric(8.0, 5.0))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("🔍").size(12.0).color(MUTED));
                                    let edit = egui::TextEdit::singleline(&mut self.search_query)
                                        .hint_text("Search name, id, units...")
                                        .frame(false)
                                        .desired_width(ui.available_width().max(60.0));
                                    ui.add(edit);
                                    ui.checkbox(&mut self.favorites_only, "★ only");
                                });
                            });

                        ui.add_space(6.0);
                        ui.separator();
                        ui.add_space(4.0);

                        egui::ScrollArea::both()
                            .id_salt("param_tree_inner")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.heading(
                                    egui::RichText::new("Calibration Tables").size(13.0).color(MUTED),
                                );
                                ui.add_space(4.0);

                                let table_count = self.xdf.tables.len();
                                for idx in 0..table_count {
                                    let table_id = self.xdf.tables[idx].id.clone();
                                    if beginner_mode
                                        && !crate::simple_ux::is_beginner_safe_table(&table_id)
                                    {
                                        continue;
                                    }
                                    let table_title = self.xdf.tables[idx].title.clone();
                                    let table_desc = self.xdf.tables[idx].description.clone();
                                    let table_units = self.xdf.tables[idx].z_units.clone();
                                    let table_values = self.xdf.tables[idx].values.clone();
                                    let y_dim = self.xdf.tables[idx].y_axis.dim;

                                    if self.favorites_only && !self.favorites.contains(&table_id) {
                                        continue;
                                    }
                                    if !param_matches_query(
                                        &self.search_query,
                                        &table_title,
                                        &table_desc,
                                        &table_id,
                                        &table_units,
                                    ) {
                                        continue;
                                    }

                                    let is_selected = self.selected_table_idx == idx;
                                    let badge = if y_dim > 1 { "[3D]" } else { "[2D]" };
                                    let edited = self
                                        .baseline_tables()
                                        .get(idx)
                                        .map(|s| table_has_edits(&table_values, s))
                                        .unwrap_or(false);
                                    let is_fav = self.favorites.contains(&table_id);

                                    ui.allocate_ui(Vec2::new(ui.available_width(), 30.0), |ui| {
                                        ui.horizontal_centered(|ui| {
                                            let star = if is_fav { "★" } else { "☆" };
                                            if ui
                                                .button(egui::RichText::new(star).size(14.0).color(
                                                    if is_fav {
                                                        Color32::from_rgb(255, 214, 0)
                                                    } else {
                                                        MUTED
                                                    },
                                                ))
                                                .clicked()
                                            {
                                                let id = table_id.clone();
                                                self.toggle_table_favorite(&id);
                                            }
                                            ui.add_space(2.0);
                                            ui.label(
                                                egui::RichText::new(badge).strong().color(ACCENT_CYAN),
                                            );
                                            let title_color = if is_selected { TEXT } else { MUTED };
                                            let title = if edited {
                                                format!("{} •", table_title)
                                            } else {
                                                table_title.clone()
                                            };
                                            if ui
                                                .selectable_label(
                                                    is_selected,
                                                    egui::RichText::new(&title)
                                                        .color(title_color)
                                                        .strong(),
                                                )
                                                .clicked()
                                            {
                                                self.selected_table_idx = idx;
                                                self.live_trace.clear();
                                                self.active_cell = (0, 0);
                                                self.selection_anchor = None;
                                            }
                                        });
                                    });
                                }

                                ui.add_space(8.0);
                                ui.separator();
                                ui.add_space(4.0);
                                ui.heading(
                                    egui::RichText::new("Scalar Constants").size(13.0).color(MUTED),
                                );
                                ui.add_space(4.0);
                                for (i, c) in self.xdf.constants.iter().enumerate() {
                                    if beginner_mode
                                        && !crate::simple_ux::is_beginner_safe_constant(&c.id)
                                    {
                                        continue;
                                    }
                                    if !param_matches_query(
                                        &self.search_query,
                                        &c.title,
                                        "",
                                        &c.id,
                                        &c.units,
                                    ) {
                                        continue;
                                    }
                                    if self.favorites_only {
                                        continue;
                                    }
                                    let basis = self.baseline_constants().get(i).copied().unwrap_or(c.value);
                                    let changed = cell_changed(c.value, basis);
                                    ui.allocate_ui(Vec2::new(ui.available_width(), 28.0), |ui| {
                                        ui.horizontal_centered(|ui| {
                                            ui.add_space(2.0);
                                            ui.label(
                                                egui::RichText::new("[1D]")
                                                    .strong()
                                                    .color(Color32::from_rgb(255, 193, 7)),
                                            );
                                            let label = if changed {
                                                format!(
                                                    "{}: {} {}",
                                                    c.title,
                                                    format_cell_display(c.value, basis),
                                                    c.units
                                                )
                                            } else {
                                                format!("{}: {:.0} {}", c.title, c.value, c.units)
                                            };
                                            ui.label(
                                                egui::RichText::new(label)
                                                    .color(TEXT)
                                                    .strong(),
                                            );
                                        });
                                    });
                                }
                            });
                    });
            });

        if !beginner_mode {
        // Right 3D Isometric Surface Mesh
        egui::SidePanel::right("surface_mesh_panel")
            .resizable(true)
            .default_width(panel_default)
            .width_range(panel_min..=panel_max)
            .frame(theme::panel_frame())
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.heading(egui::RichText::new("3D Surface Mesh").size(14.0).color(ACCENT_CYAN));
                    ui.label(
                        egui::RichText::new("Drag: orbit · Scroll: zoom · Click: select cell · Double-click: reset view")
                            .small()
                            .color(MUTED),
                    );
                });
                ui.add_space(4.0);

                let avail = ui.available_size();
                let telemetry_reserve = 118.0_f32.min(avail.y * 0.34);
                let mesh_h = (avail.y - telemetry_reserve).max(96.0);
                let mesh_side = avail.x.min(mesh_h).max(96.0);
                let mesh_size = Vec2::new(mesh_side, mesh_side);
                let (response, painter) =
                    ui.allocate_painter(mesh_size, egui::Sense::click_and_drag());
                if response.double_clicked() {
                    self.mesh_yaw = 0.785;
                    self.mesh_pitch = 0.523;
                    self.mesh_zoom = 1.0;
                }
                if response.dragged() {
                    self.mesh_yaw += response.drag_delta().x * 0.01;
                    self.mesh_pitch = (self.mesh_pitch + response.drag_delta().y * 0.01).clamp(0.1, 1.4);
                }
                if response.hovered() {
                    let scroll = ui.input(|i| i.smooth_scroll_delta.y);
                    if scroll.abs() > 0.0 {
                        let factor = 1.0 + scroll * 0.002;
                        self.mesh_zoom = (self.mesh_zoom * factor).clamp(0.45, 2.2);
                    }
                }

                if !self.xdf.tables.is_empty() {
                    let table = &self.xdf.tables[self.selected_table_idx];
                    let hover_cell = if response.dragged() {
                        None
                    } else {
                        response.hover_pos().and_then(|pos| {
                            surface_mesh::pick_cell(
                                response.rect,
                                table,
                                self.mesh_yaw,
                                self.mesh_pitch,
                                self.mesh_zoom,
                                pos,
                            )
                        })
                    };
                    surface_mesh::paint_surface_mesh(
                        &painter,
                        response.rect,
                        table,
                        self.mesh_yaw,
                        self.mesh_pitch,
                        self.mesh_zoom,
                        self.active_cell,
                        hover_cell,
                        &self.live_trace.trail(Instant::now()),
                        !self.live_trace.out_of_range.is_empty(),
                    );
                    if response.clicked() {
                        if let Some(cell) = hover_cell {
                            self.active_cell = cell;
                            self.selection_anchor = None;
                        }
                    }
                } else {
                    painter.rect_filled(response.rect, 4.0, theme::BG_FILL);
                }

                ui.add_space(6.0);
                self.paint_live_telemetry_card(ui);
            });
        }

        // Center Heatmap Table Grid
        egui::CentralPanel::default()
            .frame(theme::panel_frame())
            .show(ctx, |ui| {
                if beginner_mode {
                    ui.label(
                        egui::RichText::new(
                            "Beginner cal subset — lambda & VE tables plus rev limiter scalar only.",
                        )
                        .small()
                        .color(MUTED),
                    );
                    ui.add_space(4.0);
                }
                ui.horizontal_wrapped(|ui| {
                    if !beginner_mode && ui.button("Open ROM").on_hover_text("Open a .bin or .cuw image").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("ROM image", &["bin", "cuw"])
                            .pick_file()
                        {
                            self.load_rom_from_path(path);
                        }
                    }
                    if !beginner_mode && ui.button("Open XDF").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("TunerPro XDF", &["xdf"])
                            .pick_file()
                        {
                            self.load_xdf_from_path(path);
                        }
                    }
                    if !beginner_mode && ui.button("Open Tune").on_hover_text("Open a .wbtune (ROM + definition + notes)").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Wideband tune", &[crate::tune_file::EXTENSION])
                            .pick_file()
                        {
                            self.open_tune(path);
                        }
                    }
                    if !beginner_mode && ui.button("Save Tune").on_hover_text("Save ROM + definition reference + vehicle + notes as .wbtune").clicked() {
                        let mut dialog = rfd::FileDialog::new().add_filter("Wideband tune", &[crate::tune_file::EXTENSION]);
                        if let Some(p) = &self.rom_path {
                            if let Some(stem) = p.file_stem() {
                                dialog = dialog.set_file_name(format!("{}.{}", stem.to_string_lossy(), crate::tune_file::EXTENSION));
                            }
                        }
                        if let Some(path) = dialog.save_file() {
                            self.save_tune(path);
                        }
                    }
                    if !beginner_mode && ui.button("Compare").on_hover_text("Open a second ROM to diff against (same XDF)").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("ROM image", &["bin", "cuw"])
                            .pick_file()
                        {
                            self.load_compare_rom(path);
                        }
                    }
                    if !beginner_mode && ui.button("Save ROM").clicked() {
                        let mut dialog = rfd::FileDialog::new().add_filter("ROM image", &["bin"]);
                        if let Some(p) = &self.rom_path {
                            if let Some(name) = p.file_name() {
                                dialog = dialog.set_file_name(name.to_string_lossy().to_string());
                            }
                        }
                        if let Some(path) = dialog.save_file() {
                            self.save_rom_to_path(path);
                        }
                    }
                    if !beginner_mode {
                        let rom_label = self
                            .rom_path
                            .as_ref()
                            .and_then(|p| p.file_name())
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| "No ROM file".to_string());
                        let xdf_label = self
                            .xdf_path
                            .as_ref()
                            .and_then(|p| p.file_name())
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| self.xdf.title.clone());
                        ui.label(
                            egui::RichText::new(format!("ROM: {} | XDF: {}", rom_label, xdf_label))
                                .small()
                                .color(MUTED),
                        );
                        if !self.definition_pack_label.is_empty() {
                            ui.label(
                                egui::RichText::new(&self.definition_pack_label)
                                    .small()
                                    .color(ACCENT_CYAN),
                            );
                        }
                    }
                });
                if !beginner_mode {
                    // One toggle row instead of three stacked collapsible headers: only the open
                    // tool takes vertical space, so the grid keeps room on small windows.
                    ui.horizontal_wrapped(|ui| {
                        ui.label(egui::RichText::new("Tools").small().color(MUTED));
                        for (tool, label, tip) in [
                            (CalTool::Histogram, "Histogram", "Scanner histogram: bin any logged channel into this table"),
                            (CalTool::FuelCorrection, "Fuel correction", "VE / fuel correction from wideband + fuel trims"),
                            (CalTool::Notes, "Tune notes", "Notes saved inside the .wbtune"),
                        ] {
                            let on = self.cal_tool == Some(tool);
                            if ui.selectable_label(on, label).on_hover_text(tip).clicked() {
                                self.cal_tool = if on { None } else { Some(tool) };
                            }
                        }
                    });
                }
                if !beginner_mode {
                    self.render_compare_bar(ui);
                    match self.cal_tool {
                        Some(CalTool::Notes) => {
                            ui.add(
                                egui::TextEdit::multiline(&mut self.tune_notes)
                                    .desired_rows(3)
                                    .desired_width(f32::INFINITY)
                                    .hint_text("What changed, fuel used, mods, dyno numbers\u{2026} (saved in the .wbtune)"),
                            );
                        }
                        Some(CalTool::Histogram) => self.render_histogram_panel(ui),
                        Some(CalTool::FuelCorrection) => self.render_ve_correction_panel(ui),
                        None => {}
                    }
                }
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(6.0);

                if self.xdf.tables.is_empty() {
                    ui.label(egui::RichText::new("No calibration tables loaded.").color(MUTED));
                    return;
                }

                let table_title = self.xdf.tables[self.selected_table_idx].title.clone();
                let table_desc = self.xdf.tables[self.selected_table_idx].description.clone();
                let z_units = self.xdf.tables[self.selected_table_idx].z_units.clone();

                if self.rom_is_placeholder() {
                    egui::Frame::none()
                        .fill(Color32::from_rgb(255, 152, 0).gamma_multiply(0.12))
                        .stroke(Stroke::new(1.0, Color32::from_rgb(255, 152, 0)))
                        .rounding(egui::Rounding::same(4.0))
                        .inner_margin(egui::Margin::symmetric(8.0, 5.0))
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(
                                    "No ROM loaded - values and axes below are decoded from a blank (0xFF) buffer, not a calibration. Open a .bin/.cuw or Read Stock ROM.",
                                )
                                .small()
                                .color(Color32::from_rgb(255, 196, 120)),
                            );
                        });
                    ui.add_space(6.0);
                }

                ui.horizontal_wrapped(|ui| {
                    ui.heading(egui::RichText::new(&table_title).size(16.0).color(TEXT));
                    ui.label(egui::RichText::new(format!("({})", z_units)).color(ACCENT_CYAN));
                    if let Err(e) = self.xdf.tables[self.selected_table_idx].format.check() {
                        ui.label(
                            egui::RichText::new(format!("READ-ONLY: {}", e))
                                .small()
                                .strong()
                                .color(Color32::from_rgb(0xF8, 0x51, 0x49)),
                        );
                    }
                    let ck_color = if self.checksum_status == CHECKSUM_NOT_CONFIGURED {
                        Color32::from_rgb(255, 152, 0)
                    } else if self.checksum_status == CHECKSUM_VALID {
                        EMERALD
                    } else {
                        Color32::from_rgb(0xF5, 0x88, 0x36)
                    };
                    ui.label(
                        egui::RichText::new(&self.checksum_status)
                            .size(11.0)
                            .color(ck_color)
                            .strong(),
                    );
                });
                ui.label(egui::RichText::new(&table_desc).small().color(MUTED));
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(6.0);

                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new("Math Step:").strong().color(TEXT));
                    // Wide range so "=" can set any engineering value (RPM limits, negative timing).
                    ui.add(egui::DragValue::new(&mut self.math_amount).speed(0.5).range(-100000.0..=100000.0));
                    let amt = self.math_amount;

                    if ui.button("  +  ").on_hover_text("Add step to selection").clicked() {
                        self.run_table_math(MathOp::Add(amt));
                    }
                    if ui.button("  -  ").on_hover_text("Subtract step from selection").clicked() {
                        self.run_table_math(MathOp::Sub(amt));
                    }
                    if ui.button(" +% ").on_hover_text("Increase selection by step %").clicked() {
                        self.run_table_math(MathOp::Percent(amt));
                    }
                    if ui.button(" -% ").on_hover_text("Decrease selection by step %").clicked() {
                        self.run_table_math(MathOp::Percent(-amt));
                    }
                    if ui.button("  =  ").on_hover_text("Set selection to step").clicked() {
                        self.run_table_math(MathOp::Set(amt));
                    }
                    if !beginner_mode {
                        ui.separator();
                        if ui.button("Interp H").on_hover_text("Linear fill across each selected row").clicked() {
                            self.edit_selected_table("interpolate H", table_edit::interpolate_horizontal);
                        }
                        if ui.button("Interp V").on_hover_text("Linear fill down each selected column").clicked() {
                            self.edit_selected_table("interpolate V", table_edit::interpolate_vertical);
                        }
                        if ui.button("Interp 2D").on_hover_text("Bilinear fill from the selection's 4 corners").clicked() {
                            self.edit_selected_table("interpolate 2D", table_edit::interpolate_2d);
                        }
                        if ui.button("Smooth").on_hover_text("Smooth selection (whole table if one cell)").clicked() {
                            if self.selection().is_single() {
                                let (rows, cols) = {
                                    let t = &self.xdf.tables[self.selected_table_idx];
                                    (t.values.len(), t.values.first().map_or(0, |r| r.len()))
                                };
                                self.selection_anchor = Some((0, 0));
                                self.active_cell = (rows.saturating_sub(1), cols.saturating_sub(1));
                            }
                            self.edit_selected_table("smooth", table_edit::smooth);
                        }
                    }
                    ui.separator();
                    let undo_tip = self
                        .table_undo
                        .undo_label()
                        .map(|l| format!("Undo {} (Ctrl+Z)", l))
                        .unwrap_or_else(|| "Nothing to undo".to_string());
                    if ui
                        .add_enabled(self.table_undo.can_undo(), egui::Button::new("Undo"))
                        .on_hover_text(undo_tip)
                        .clicked()
                    {
                        self.undo_table_edit();
                    }
                    if ui
                        .add_enabled(self.table_undo.can_redo(), egui::Button::new("Redo"))
                        .on_hover_text("Redo (Ctrl+Y / Ctrl+Shift+Z)")
                        .clicked()
                    {
                        self.redo_table_edit();
                    }
                    egui::ComboBox::from_id_salt("paste_mode")
                        .selected_text(format!("Ctrl+V: {}", self.paste_mode.label()))
                        .show_ui(ui, |ui| {
                            for m in [table_edit::PasteMode::Replace, table_edit::PasteMode::Add, table_edit::PasteMode::MultiplyPercent] {
                                ui.selectable_value(&mut self.paste_mode, m, m.label());
                            }
                        });
                    let sel = self.selection();
                    let (r0, r1, c0, c1) = sel.bounds();
                    ui.label(
                        egui::RichText::new(if sel.is_single() {
                            format!("Cell R{} C{}", r0, c0)
                        } else {
                            format!("{} cells (R{}-{} C{}-{})", sel.count(), r0, r1, c0, c1)
                        })
                        .small()
                        .color(MUTED),
                    );
                    if let Some(buf) = &self.cell_entry {
                        ui.label(
                            egui::RichText::new(format!("Set to: {}\u{258F}  (Enter = apply, Esc = cancel)", buf))
                                .monospace()
                                .strong()
                                .color(ACCENT_CYAN),
                        );
                    } else {
                        ui.label(egui::RichText::new("\u{2328}").color(MUTED)).on_hover_text(
                            "Type a value + Enter to set the selection\n+ - * / : step, -step, +step %, -step %\nArrows move, Shift+arrows extend\nCtrl+C / Ctrl+V copy & paste (Excel)\nCtrl+Z / Ctrl+Y undo & redo",
                        );
                    }
                });

                self.handle_grid_keyboard(ui);
                ui.add_space(8.0);

                let grid_avail = ui.available_size();
                egui::ScrollArea::both()
                    .id_salt("tune_grid")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_min_size(Vec2::new(grid_avail.x.max(200.0), grid_avail.y.max(120.0)));
                        self.render_table_grid(ui);
                    });
            });
    }

    fn render_table_grid(&mut self, ui: &mut egui::Ui) {
        let table = &self.xdf.tables[self.selected_table_idx];
        let stock = &self.baseline_tables()[self.selected_table_idx].clone();
        let x_labels = table.x_axis.labels.clone();
        let y_labels = table.y_axis.labels.clone();
        let rows = table.values.len();
        let cols = table.values[0].len();
        let live_cell = self.live_trace.current().map(live_trace::nearest_cell);
        let sel = self.selection().clamped(rows, cols);
        let multi = !sel.is_single();
        let hist_overlay = self.hist.overlay_for(&self.xdf.tables[self.selected_table_idx]);
        let corr_overlay_owned = if hist_overlay.is_some() {
            None
        } else {
            self.ve_corr.overlay_for(self.selected_table_idx).cloned()
        };
        let corr_overlay = corr_overlay_owned.as_ref();
        if !ui.input(|i| i.pointer.primary_down()) {
            self.grid_drag_select = false;
        }

        // Calculate Min/Max for heatmap
        let mut min_val = f64::MAX;
        let mut max_val = f64::MIN;
        for r in 0..rows {
            for c in 0..cols {
                let v = table.values[r][c];
                min_val = min_val.min(v);
                max_val = max_val.max(v);
            }
        }

        let num_cols = cols + 1; // +1 for Y header
        let table_builder = TableBuilder::new(ui)
            .striped(false)
            .cell_layout(egui::Layout::centered_and_justified(egui::Direction::LeftToRight))
            .columns(Column::initial(70.0).resizable(true), num_cols);

        table_builder
            .header(28.0, |mut header| {
                header.col(|ui| {
                    let rect = ui.max_rect();
                    ui.painter().rect_filled(rect, 0.0, theme::TABLE_HEADER);
                    ui.label(egui::RichText::new("Y / X").strong().monospace().color(MUTED));
                });
                for c in 0..cols {
                    let lbl = if c < x_labels.len() { x_labels[c] } else { (c * 500) as f64 };
                    header.col(|ui| {
                        let rect = ui.max_rect();
                        ui.painter().rect_filled(rect, 0.0, theme::TABLE_HEADER);
                        ui.label(
                            egui::RichText::new(format!("{:>6.0}", lbl))
                                .monospace()
                                .strong()
                                .color(ACCENT_CYAN),
                        );
                    });
                }
            })
            .body(|mut body| {
                for r in 0..rows {
                    body.row(24.0, |mut row| {
                        // Y-Axis Row Header
                        let y_lbl = if r < y_labels.len() { y_labels[r] } else { (r * 10) as f64 };
                        row.col(|ui| {
                            let rect = ui.max_rect();
                            ui.painter().rect_filled(rect, 0.0, theme::TABLE_HEADER);
                            ui.label(
                                egui::RichText::new(format!("{:>5.0}", y_lbl))
                                    .monospace()
                                    .strong()
                                    .color(ACCENT_CYAN),
                            );
                        });

                        // Cell values with Heatmap color + Gold delta
                        for c in 0..cols {
                            let val = self.xdf.tables[self.selected_table_idx].values[r][c];
                            let stock_val = if r < stock.len() && c < stock[r].len() { stock[r][c] } else { val };
                            let is_delta = cell_changed(val, stock_val);
                            let is_selected = self.active_cell == (r, c);
                            let is_live = live_cell == Some((r, c));

                            let mut bg_color = heatmap_color(val, min_val, max_val);
                            let text_color = Color32::from_rgb(0xFF, 0xFF, 0xFF);
                            let mut cell_text = format_cell_display(val, stock_val);
                            let mut corr_tip: Option<String> = None;
                            if let Some((h, lo, hi)) = &hist_overlay {
                                // Histogram view: heat across the histogram's own range.
                                match h.value(r, c) {
                                    Some(hv) => {
                                        bg_color = heatmap_color(hv, *lo, *hi);
                                        cell_text = histogram_cell_text(hv);
                                        corr_tip = Some(format!(
                                            "{} {} = {}\n{} hits \u{00B7} table value {:.2}",
                                            h.spec.aggregate.label(),
                                            h.spec.value_channel,
                                            histogram_cell_text(hv),
                                            h.hits(r, c),
                                            val
                                        ));
                                    }
                                    None => {
                                        bg_color = Color32::from_rgb(0x1E, 0x23, 0x2A);
                                        cell_text = String::new();
                                        if h.hits(r, c) > 0 {
                                            corr_tip = Some(format!("{} hits (< {} minimum)", h.hits(r, c), h.spec.min_hits));
                                        }
                                    }
                                }
                            } else if let Some(res) = corr_overlay {
                                // Suggestion view: red = lean (needs more), blue = rich (needs less).
                                match res.change_pct(r, c) {
                                    Some(p) => {
                                        let k = (p.abs() / res.max_change_pct.max(1.0)).min(1.0) as f32;
                                        let hot = if p >= 0.0 {
                                            Color32::from_rgb(0xD9, 0x3A, 0x2B)
                                        } else {
                                            Color32::from_rgb(0x2F, 0x6F, 0xE0)
                                        };
                                        bg_color = lerp_color(Color32::from_rgb(0x2A, 0x30, 0x38), hot, 0.25 + 0.75 * k);
                                        // Avoid "-0.0%" for tiny negatives.
                                        let shown = if p.abs() < 0.05 { 0.0 } else { p };
                                        cell_text = format!("{:+.1}%", shown);
                                        let s = res.stats[r][c];
                                        corr_tip = Some(format!(
                                            "{:.2} -> {:.2}\n{} samples, weight {:.1}",
                                            val,
                                            val * (1.0 + p / 100.0),
                                            s.hits,
                                            s.weight
                                        ));
                                    }
                                    None => {
                                        bg_color = Color32::from_rgb(0x1E, 0x23, 0x2A);
                                        let s = res.stats.get(r).and_then(|row| row.get(c)).copied().unwrap_or_default();
                                        if s.hits > 0 {
                                            corr_tip = Some(format!("Not enough data: weight {:.1} < {:.1}", s.weight, res.min_weight));
                                        }
                                    }
                                }
                            }

                            row.col(|ui| {
                                let rect = ui.available_rect_before_wrap();
                                let painter = ui.painter();

                                painter.rect_filled(rect, 2.0, bg_color);

                                if is_delta {
                                    painter.rect_stroke(
                                        rect,
                                        2.0,
                                        Stroke::new(1.5, Color32::from_rgb(255, 214, 0)),
                                    );
                                }

                                if multi && sel.contains((r, c)) {
                                    // Wash + outline: a faint tint alone vanished on teal/blue cells.
                                    painter.rect_filled(rect, 2.0, Color32::from_rgba_unmultiplied(255, 255, 255, 48));
                                    painter.rect_stroke(rect.shrink(1.0), 2.0, Stroke::new(1.5, ACCENT_CYAN));
                                }
                                if is_live {
                                    theme::paint_operating_cell_glow(painter, rect);
                                }
                                if is_selected {
                                    paint_live_cell_glow(painter, rect);
                                }

                                let response = ui.selectable_label(
                                    is_selected,
                                    egui::RichText::new(cell_text)
                                        .monospace()
                                        .strong()
                                        .color(text_color),
                                );
                                let response = response.interact(egui::Sense::click_and_drag());
                                let response = match &corr_tip {
                                    Some(t) => response.on_hover_text(t.as_str()),
                                    None => response,
                                };
                                if response.drag_started() {
                                    self.grid_drag_select = true;
                                    self.selection_anchor = Some((r, c));
                                    self.active_cell = (r, c);
                                } else if self.grid_drag_select && ui.rect_contains_pointer(response.rect) {
                                    self.active_cell = (r, c);
                                }
                                if response.double_clicked() {
                                    self.selection_anchor = None;
                                    self.active_cell = (r, c);
                                    self.cell_entry = Some(format!("{}", val));
                                }
                                if response.clicked() {
                                    if ui.input(|i| i.modifiers.shift) {
                                        // Extend from the current anchor (or the old cursor).
                                        self.selection_anchor = Some(self.selection_anchor.unwrap_or(self.active_cell));
                                    } else {
                                        self.selection_anchor = None;
                                    }
                                    self.active_cell = (r, c);
                                }
                            });
                        }
                    });
                }
            });
    }

}

// ---------------------------------------------------------------------------
// Tab 2: Telemetry & Multi-Trace Oscilloscope Strip Chart
// ---------------------------------------------------------------------------

impl WidebandApp {
    pub(crate) fn render_telemetry_tab(&mut self, ctx: &egui::Context, beginner_mode: bool) {
        let channel_defs = self.telemetry.lock().unwrap().channels.clone();

        let (gauge_values, recording, def_title, stats_line, rate_rows) = {
            let buf = self.telemetry.lock().unwrap();
            // (label, unit, value) per dashboard gauge; None when the channel isn't defined.
            let gauges: Vec<Option<(String, String, f32)>> = self
                .dashboard
                .gauges
                .iter()
                .map(|g| {
                    let i = buf.channel_idx(&g.channel)?;
                    let ch = &buf.channels[i];
                    // Never answered (unsupported / placeholder PID) => no value, not a fake 0.
                    let v = if buf.channel_rates[i].has_sampled() { buf.latest[i] } else { f32::NAN };
                    Some((ch.label.clone(), ch.unit.clone(), v))
                })
                .collect();
            let tick_ms = buf.scheduler.settings().tick_ms;
            let budget = buf.scheduler.settings().max_requests_per_tick;
            let stats_line = format!(
                "Scheduler {} ms tick · {} req/tick · ring {} · drops ring {} rec {} skip {} bus {}",
                tick_ms,
                budget,
                buf.max_capacity,
                buf.ring_drop_count,
                buf.record_drop_count,
                buf.total_scheduler_skips(),
                buf.bus_fail_count
            );
            let rate_rows: Vec<(String, f32, u64)> = buf
                .channels
                .iter()
                .enumerate()
                .map(|(i, ch)| {
                    (
                        ch.label.clone(),
                        buf.channel_rates[i].hz,
                        buf.scheduler.per_channel_budget_skips[i],
                    )
                })
                .collect();
            (gauges, buf.is_recording, buf.definition_title.clone(), stats_line, rate_rows)
        };

        let live = self.is_connected && self.live_poller_active;
        egui::TopBottomPanel::top("gauge_strip").min_height(72.0).show(ctx, |ui| {
            // Buttons on their own row: a right-to-left layout sharing the gauge row used to take
            // the whole width and hide every gauge.
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Dashboard").strong().color(TEXT));
                if !live {
                    ui.label(egui::RichText::new("(not polling - showing last values)").small().color(MUTED));
                }
                if !beginner_mode {
                    ui.toggle_value(&mut self.dash_editing, "Edit gauges");
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !beginner_mode && ui.button("Export CSV").clicked() {
                        let path = "datalog.csv".to_string();
                        let tele = Arc::clone(&self.telemetry);
                        let has_data = tele.lock().map(|b| !b.frames.is_empty()).unwrap_or(false);
                        if has_data {
                            TelemetryBuffer::export_csv_async(path, tele);
                            self.log("Datalog export started (background thread, datalog.csv).");
                        } else {
                            self.log("Export failed — start live polling or record first.");
                        }
                    }
                    let rec_label = if recording { "■ Stop" } else { "● Record" };
                    if ui.button(rec_label).clicked() {
                        let on = {
                            let mut buf = self.telemetry.lock().unwrap();
                            let next = !buf.is_recording;
                            buf.set_recording(next);
                            next
                        };
                        self.log(if on {
                            "Telemetry recording started (samples appended to record buffer)."
                        } else {
                            "Telemetry recording stopped."
                        });
                    }
                    if !beginner_mode && ui.button("Clear live").clicked() {
                        self.telemetry.lock().unwrap().clear();
                        self.log("Telemetry history cleared.");
                    }
                });
            });
            if self.dash_editing {
                self.render_dashboard_editor(ui, &channel_defs);
            }
            ui.horizontal_wrapped(|ui| {
                for (spec, v) in self.dashboard.gauges.iter().zip(gauge_values.iter()) {
                    match v {
                        Some((label, unit, value)) => paint_gauge(ui, spec, label, unit, *value as f64, live),
                        None => paint_gauge(ui, spec, &spec.channel, "not defined", f64::NAN, false),
                    }
                }
            });
        });

        if !beginner_mode {
        egui::SidePanel::left("telemetry_playback_panel")
            .default_width(280.0)
            .min_width(220.0)
            .show(ctx, |ui| {
                ui.heading(egui::RichText::new("Log playback").size(14.0));
                ui.separator();

                ui.horizontal(|ui| {
                    ui.label("View:");
                    egui::ComboBox::from_id_salt("tel_view_mode")
                        .selected_text(Self::view_mode_label(self.telemetry_playback.mode))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.telemetry_playback.mode,
                                TelemetryViewMode::Live,
                                "Live",
                            );
                            ui.selectable_value(
                                &mut self.telemetry_playback.mode,
                                TelemetryViewMode::Playback,
                                "Playback",
                            );
                            ui.selectable_value(
                                &mut self.telemetry_playback.mode,
                                TelemetryViewMode::CompareOverlay,
                                "Compare overlay",
                            );
                            ui.selectable_value(
                                &mut self.telemetry_playback.mode,
                                TelemetryViewMode::CompareSideBySide,
                                "Compare side-by-side",
                            );
                        });
                });

                if ui.button("Open CSV log…").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("CSV", &["csv"])
                        .pick_file()
                    {
                        match import_log_csv(&path, &channel_defs) {
                            Ok(log) => {
                                self.telemetry_playback.set_primary_log(log);
                                let msg = self.telemetry_playback.status_message.clone();
                                self.log(&msg);
                            }
                            Err(e) => self.log(&format!("Log import failed: {}", e)),
                        }
                    }
                }
                if ui.button("Open compare log…").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("CSV", &["csv"])
                        .pick_file()
                    {
                        match import_log_csv(&path, &channel_defs) {
                            Ok(log) => {
                                self.telemetry_playback.set_compare_log(log);
                                let msg = self.telemetry_playback.status_message.clone();
                                self.log(&msg);
                            }
                            Err(e) => self.log(&format!("Compare import failed: {}", e)),
                        }
                    }
                }

                if self.telemetry_playback.primary.is_some() {
                    let frame_max = self
                        .telemetry_playback
                        .primary
                        .as_ref()
                        .map(|l| l.frames.len().saturating_sub(1))
                        .unwrap_or(0);
                    let mut scrub = self.telemetry_playback.scrub_frame.min(frame_max) as f32;
                    ui.add(
                        egui::Slider::new(&mut scrub, 0.0..=frame_max as f32)
                            .text("Scrub")
                            .smart_aim(false),
                    );
                    self.telemetry_playback.scrub_frame = scrub as usize;
                    self.telemetry_playback.clamp_scrub();
                    ui.label(
                        egui::RichText::new(format!(
                            "t = {:.2}s · frame {}/{}",
                            self.telemetry_playback.scrub_time_sec(),
                            self.telemetry_playback.scrub_frame + 1,
                            frame_max + 1
                        ))
                        .small()
                        .monospace(),
                    );
                    ui.horizontal(|ui| {
                        let play_label = if self.telemetry_playback.playing { "⏸ Pause" } else { "▶ Play" };
                        if ui.button(play_label).clicked() {
                            self.telemetry_playback.playing = !self.telemetry_playback.playing;
                        }
                        ui.add(
                            egui::DragValue::new(&mut self.telemetry_playback.play_speed)
                                .speed(0.05)
                                .range(0.25..=4.0)
                                .suffix("×"),
                        );
                    });
                }

                ui.separator();
                ui.label(egui::RichText::new("Markers").strong());
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.telemetry_playback.marker_label_draft)
                            .hint_text("Label"),
                    );
                });
                ui.add(
                    egui::TextEdit::multiline(&mut self.telemetry_playback.marker_comment_draft)
                        .desired_rows(2)
                        .hint_text("Comment"),
                );
                if ui.button("Add marker at scrub").clicked() {
                    self.telemetry_playback.add_marker_at_scrub();
                    let msg = self.telemetry_playback.status_message.clone();
                    self.log(&msg);
                }
                if let Some(log) = &self.telemetry_playback.primary {
                    for (i, m) in log.markers.iter().enumerate() {
                        let sel = self.telemetry_playback.selected_marker == Some(i);
                        if ui
                            .selectable_label(sel, format!("{:.2}s — {}", m.time_sec, m.label))
                            .clicked()
                        {
                            self.telemetry_playback.selected_marker = Some(i);
                        }
                        if sel && !m.comment.is_empty() {
                            ui.label(egui::RichText::new(&m.comment).small().color(MUTED));
                        }
                    }
                }

                ui.separator();
                ui.label(egui::RichText::new("Math channels").strong());
                ui.add(
                    egui::TextEdit::singleline(&mut self.telemetry_playback.formula_draft)
                        .hint_text("e.g. lambda / target"),
                );
                if ui.button("Add math channel").clicked() {
                    let id = format!("math_{}", self.telemetry_playback.math_channels.len() + 1);
                    self.telemetry_playback.math_channels.push(MathChannelDef {
                        id,
                        label: self.telemetry_playback.formula_draft.clone(),
                        formula: self.telemetry_playback.formula_draft.clone(),
                        unit: "".into(),
                        color: [200, 120, 255],
                    });
                }
                for ch in &self.telemetry_playback.math_channels {
                    ui.label(
                        egui::RichText::new(format!("{} = {}", ch.label, ch.formula))
                            .small()
                            .monospace(),
                    );
                }
                if let (Some(log), Some(idx)) = (
                    self.telemetry_playback.primary.as_ref(),
                    self.telemetry_playback.primary.as_ref().map(|_| self.telemetry_playback.scrub_frame),
                ) {
                    if let Some(math) = self.telemetry_playback.math_channels.first() {
                        if let Some(v) = log.eval_math(math, idx, &channel_defs) {
                            ui.label(
                                egui::RichText::new(format!("@ scrub: {:.4}", v))
                                    .small()
                                    .color(ACCENT_CYAN),
                            );
                        }
                    }
                }

                if !self.telemetry_playback.status_message.is_empty() {
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(&self.telemetry_playback.status_message)
                            .small()
                            .color(MUTED),
                    );
                }
            });
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            let mode = if beginner_mode {
                TelemetryViewMode::Live
            } else {
                self.telemetry_playback.mode
            };
            let title = match mode {
                TelemetryViewMode::Live => "Definition-Driven Live Strip Chart",
                TelemetryViewMode::Playback => "CSV Log Playback",
                TelemetryViewMode::CompareOverlay => "Multi-Log Compare (overlay)",
                TelemetryViewMode::CompareSideBySide => "Multi-Log Compare (side-by-side)",
            };
            ui.horizontal_wrapped(|ui| {
                ui.heading(egui::RichText::new(title).size(15.0).color(Color32::WHITE));
                ui.label(
                    egui::RichText::new(format!("({})", def_title))
                        .small()
                        .color(Color32::GRAY),
                );
            });
            if !beginner_mode && mode == TelemetryViewMode::Live {
                ui.label(egui::RichText::new(stats_line).size(11.0).monospace().color(MUTED));
                ui.horizontal_wrapped(|ui| {
                    for (label, hz, skips) in &rate_rows {
                        let color = if *skips > 0 {
                            Color32::from_rgb(0xF8, 0x51, 0x49)
                        } else {
                            ACCENT_CYAN
                        };
                        ui.label(
                            egui::RichText::new(format!("{} {:.1} Hz", label, hz))
                                .size(10.0)
                                .monospace()
                                .color(color),
                        );
                        if *skips > 0 {
                            ui.label(
                                egui::RichText::new(format!("(-{})", skips))
                                    .size(10.0)
                                    .color(Color32::from_rgb(0xF8, 0x51, 0x49)),
                            );
                        }
                        ui.add_space(8.0);
                    }
                });
            }
            ui.add_space(4.0);
            if beginner_mode {
                if ui.button("Next: Safe cal edits →").clicked() {
                    self.simple_ux.beginner_step = BeginnerStep::Calibrate;
                    self.sync_beginner_step_to_tab();
                }
                ui.add_space(4.0);
            }

            let plot_h = ui.available_size().y.max(160.0);
            let math_channels = self.telemetry_playback.math_channels.clone();

            match mode {
                TelemetryViewMode::Live => {
                    let buffer = self.telemetry.lock().unwrap();
                    let lines = Self::build_channel_plot_lines(
                        &buffer.channels,
                        buffer.frames.iter().collect(),
                        &math_channels,
                        &channel_defs,
                        None,
                        None,
                    );
                    Self::paint_telemetry_plot(ui, "telemetry_oscilloscope", plot_h, lines, &[]);
                }
                TelemetryViewMode::Playback => {
                    if let Some(log) = self.telemetry_playback.primary.as_ref() {
                        let end = self.telemetry_playback.scrub_frame + 1;
                        let slice: Vec<&TelemetryFrame> = log.frames.iter().take(end).collect();
                        let markers = log.markers.clone();
                        let lines = Self::build_channel_plot_lines(
                            &channel_defs,
                            slice,
                            &math_channels,
                            &channel_defs,
                            None,
                            Some(log),
                        );
                        let playhead = log.frames.get(self.telemetry_playback.scrub_frame).map(|f| f.time_sec);
                        Self::paint_telemetry_plot(ui, "telemetry_playback", plot_h, lines, &markers);
                        if let Some(t) = playhead {
                            ui.label(
                                egui::RichText::new(format!("Playhead {:.2}s", t))
                                    .small()
                                    .monospace()
                                    .color(ACCENT_CYAN),
                            );
                        }
                    } else {
                        ui.label("Open a CSV log to start playback.");
                    }
                }
                TelemetryViewMode::CompareOverlay | TelemetryViewMode::CompareSideBySide => {
                    let primary = self.telemetry_playback.primary.clone();
                    let compare = self.telemetry_playback.compare.clone();
                    if primary.is_none() && compare.is_none() {
                        ui.label("Load a primary log and optional compare log.");
                    } else if mode == TelemetryViewMode::CompareSideBySide {
                        ui.columns(2, |cols| {
                            if let Some(log) = &primary {
                                cols[0].label(&log.name);
                                let frames: Vec<&TelemetryFrame> = log.frames.iter().collect();
                                let lines = Self::build_channel_plot_lines(
                                    &channel_defs,
                                    frames,
                                    &math_channels,
                                    &channel_defs,
                                    None,
                                    Some(log),
                                );
                                Self::paint_telemetry_plot(
                                    &mut cols[0],
                                    "compare_a",
                                    plot_h * 0.45,
                                    lines,
                                    &log.markers,
                                );
                            }
                            if let Some(log) = &compare {
                                cols[1].label(&log.name);
                                let frames: Vec<&TelemetryFrame> = log.frames.iter().collect();
                                let lines = Self::build_channel_plot_lines(
                                    &channel_defs,
                                    frames,
                                    &math_channels,
                                    &channel_defs,
                                    Some([255, 120, 80]),
                                    Some(log),
                                );
                                Self::paint_telemetry_plot(
                                    &mut cols[1],
                                    "compare_b",
                                    plot_h * 0.45,
                                    lines,
                                    &[],
                                );
                            }
                        });
                    } else {
                        let mut lines = Vec::new();
                        if let Some(log) = &primary {
                            let frames: Vec<&TelemetryFrame> = log.frames.iter().collect();
                            lines.extend(Self::build_channel_plot_lines(
                                &channel_defs,
                                frames,
                                &math_channels,
                                &channel_defs,
                                None,
                                Some(log),
                            ));
                        }
                        if let Some(log) = &compare {
                            let frames: Vec<&TelemetryFrame> = log.frames.iter().collect();
                            lines.extend(Self::build_channel_plot_lines(
                                &channel_defs,
                                frames,
                                &[],
                                &channel_defs,
                                Some([255, 120, 80]),
                                Some(log),
                            ));
                        }
                        let markers = primary
                            .as_ref()
                            .map(|l| l.markers.clone())
                            .unwrap_or_default();
                        Self::paint_telemetry_plot(ui, "compare_overlay", plot_h, lines, &markers);
                    }
                }
            }
        });
    }

    fn view_mode_label(mode: TelemetryViewMode) -> &'static str {
        match mode {
            TelemetryViewMode::Live => "Live",
            TelemetryViewMode::Playback => "Playback",
            TelemetryViewMode::CompareOverlay => "Compare overlay",
            TelemetryViewMode::CompareSideBySide => "Compare side-by-side",
        }
    }

    fn build_channel_plot_lines(
        channel_defs: &[crate::telemetry::TelemetryChannelDef],
        frames: Vec<&TelemetryFrame>,
        math_channels: &[MathChannelDef],
        eval_defs: &[crate::telemetry::TelemetryChannelDef],
        color_override: Option<[u8; 3]>,
        import_log: Option<&ImportedLog>,
    ) -> Vec<Line> {
        let mut plot_lines: Vec<Line> = Vec::new();
        for (ch_idx, ch) in channel_defs.iter().enumerate() {
            if ch.plot.is_none() && color_override.is_none() {
                continue;
            }
            let scale = ch.plot_scale();
            let mut points = Vec::new();
            for frame in &frames {
                if ch_idx < frame.values.len() {
                    points.push([frame.time_sec, frame.values[ch_idx] as f64 * scale]);
                }
            }
            if points.is_empty() {
                continue;
            }
            let rgb = color_override.unwrap_or_else(|| ch.plot_color());
            let name = if scale != 1.0 {
                format!("{} (×{})", ch.label, scale)
            } else {
                ch.label.clone()
            };
            plot_lines.push(
                Line::new(PlotPoints::new(points))
                    .color(Color32::from_rgb(rgb[0], rgb[1], rgb[2]))
                    .name(name),
            );
        }
        if !frames.is_empty() && !math_channels.is_empty() {
            let fallback = ImportedLog {
                name: "plot".into(),
                path: None,
                frames: frames.iter().map(|f| (*f).clone()).collect(),
                markers: Vec::new(),
                extra_values: HashMap::new(),
            };
            let log_ref = import_log.unwrap_or(&fallback);
            for math in math_channels {
                let mut points = Vec::new();
                for (fi, frame) in frames.iter().enumerate() {
                    if let Some(v) = log_ref.eval_math(math, fi, eval_defs) {
                        points.push([frame.time_sec, v]);
                    }
                }
                if !points.is_empty() {
                    plot_lines.push(
                        Line::new(PlotPoints::new(points))
                            .color(Color32::from_rgb(math.color[0], math.color[1], math.color[2]))
                            .name(math.label.clone()),
                    );
                }
            }
        }
        plot_lines
    }

    fn paint_telemetry_plot(
        ui: &mut egui::Ui,
        id: &str,
        plot_h: f32,
        lines: Vec<Line>,
        markers: &[LogMarker],
    ) {
        Plot::new(id)
            .height(plot_h)
            .width(ui.available_width())
            .view_aspect(ui.available_width() / plot_h.max(1.0))
            .show_axes([true, true])
            .legend(egui_plot::Legend::default())
            .show(ui, |plot_ui: &mut PlotUi| {
                for line in lines {
                    plot_ui.line(line);
                }
                for m in markers {
                    let x = m.time_sec;
                    plot_ui.line(
                        Line::new(PlotPoints::new(vec![[x, 0.0], [x, 1.0e6]]))
                            .color(Color32::from_rgb(255, 214, 0))
                            .name(m.label.clone()),
                    );
                }
            });
    }

}

fn yes_no(v: bool) -> &'static str {
    if v { "Yes" } else { "No" }
}

fn read_pass_thru_vbatt(dev: &mut dyn PassThruDevice, channel: u32) -> Option<f32> {
    if let Ok(out) = dev.ioctl(channel, READ_VBATT, &[]) {
        if out.len() >= 4 {
            let mv = u32::from_le_bytes([out[0], out[1], out[2], out[3]]);
            if mv > 8000 && mv < 20000 {
                return Some(mv as f32 / 1000.0);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tab: Transmission (TCM read / parameter scaffolding)
// ---------------------------------------------------------------------------

impl WidebandApp {
    fn render_transmission_tab(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.heading(
                    egui::RichText::new("Transmission Control Module (TCM)")
                        .size(16.0)
                        .color(Color32::WHITE),
                );
                ui.label(
                    egui::RichText::new(
                        "Read-oriented TCM identify and definition-tagged shift / pressure / TCC views. Flash stays fail-closed until security-config and flash-layout are configured like ECM.",
                    )
                    .small()
                    .color(MUTED),
                );
                ui.separator();

                let tcm = self.selected_tcm_profile().clone();
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new("TCM profile").size(11.0).color(MUTED));
                    egui::ComboBox::from_id_salt("tcm_profile_select")
                        .width(280.0)
                        .selected_text(&tcm.name)
                        .show_ui(ui, |ui| {
                            for (idx, prof) in self.tcm_profiles.iter().enumerate() {
                                ui.selectable_value(&mut self.selected_tcm_profile_idx, idx, &prof.name);
                            }
                        });
                });
                ui.label(
                    egui::RichText::new(format!(
                        "Paired ECM: {} · Family: {} · TX 0x{:X} / RX 0x{:X}",
                        tcm.paired_ecm_profile_id,
                        tcm.transmission_family,
                        tcm.request_id,
                        tcm.response_id
                    ))
                    .small()
                    .color(MUTED),
                );
                ui.add_space(8.0);

                ui.horizontal_wrapped(|ui| {
                    if ui
                        .button(egui::RichText::new("Identify TCM").strong())
                        .clicked()
                    {
                        self.tcm_identify_error.clear();
                        if let Some(dev) = &mut self.active_device {
                            let allow_demo = self.active_device_virtual;
                            let channel = dev.channel_id();
                            match TcmIdentifier::query(dev.as_mut(), channel, &tcm, allow_demo) {
                                Ok(id) => {
                                    let details = id.details.clone();
                                    self.tcm_id = id;
                                    self.log(&details);
                                }
                                Err(e) => {
                                    self.tcm_identify_error = e.clone();
                                    self.log(&format!("TCM identification failed: {}", e));
                                }
                            }
                        } else {
                            self.tcm_identify_error = "Connect cable first.".to_string();
                        }
                    }

                    let virtual_ecu = self.cable_is_virtual();
                    let blockers = tcm_flash_blockers(&tcm, virtual_ecu, &self.flash_layout);
                    let flash_blocked = !blockers.is_empty();
                    if ui
                        .add_enabled(
                            !flash_blocked,
                            egui::Button::new(
                                egui::RichText::new("Flash TCM calibration")
                                    .strong()
                                    .color(Color32::from_rgb(255, 120, 80)),
                            ),
                        )
                        .clicked()
                    {
                        self.tcm_flash_confirm_open = true;
                    }
                });

                if !self.tcm_identify_error.is_empty() {
                    ui.label(
                        egui::RichText::new(&self.tcm_identify_error)
                            .small()
                            .color(Color32::from_rgb(255, 152, 0)),
                    );
                }

                if !self.tcm_id.details.is_empty() {
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new(&self.tcm_id.details).monospace().color(TEXT));
                }

                if self.tcm_flash_confirm_open {
                    let virtual_ecu = self.cable_is_virtual();
                    let blockers = tcm_flash_blockers(&tcm, virtual_ecu, &self.flash_layout);
                    let mut confirm = false;
                    let mut cancel = false;
                    egui::Window::new("Confirm TCM calibration flash")
                        .collapsible(false)
                        .resizable(false)
                        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                        .show(ctx, |ui| {
                            ui.label(
                                "TCM writes require the same security-config and checksum setup as ECM. This build keeps TCM flash gated until layout and bench validation are complete.",
                            );
                            ui.label(format!("TCM profile: {}", tcm.name));
                            for reason in &blockers {
                                ui.label(
                                    egui::RichText::new(format!("Blocked: {}", reason))
                                        .color(Color32::from_rgb(255, 152, 0)),
                                );
                            }
                            ui.horizontal(|ui| {
                                if ui.button("Cancel").clicked() {
                                    cancel = true;
                                }
                                if ui
                                    .add_enabled(blockers.is_empty(), egui::Button::new("Flash"))
                                    .clicked()
                                {
                                    confirm = true;
                                }
                            });
                        });
                    if cancel {
                        self.tcm_flash_confirm_open = false;
                    }
                    if confirm && blockers.is_empty() {
                        self.tcm_flash_confirm_open = false;
                        self.log("TCM flash refused: execution path not enabled in this milestone.");
                    }
                }

                ui.add_space(12.0);
                ui.heading(egui::RichText::new("Shift / pressure / TCC (from XDF)").size(14.0).color(ACCENT_CYAN));
                ui.label(
                    egui::RichText::new(format!(
                        "{} — {}",
                        self.tcm_param_view.definition_title,
                        self.tcm_param_view.note
                    ))
                    .small()
                    .color(MUTED),
                );

                for group in [
                    TcmParameterGroup::Shift,
                    TcmParameterGroup::Pressure,
                    TcmParameterGroup::Tcc,
                ] {
                    let rows: Vec<_> = self
                        .tcm_param_view
                        .entries
                        .iter()
                        .filter(|e| e.group == group)
                        .collect();
                    if rows.is_empty() {
                        continue;
                    }
                    ui.add_space(8.0);
                    ui.collapsing(group_label(group), |ui| {
                        TableBuilder::new(ui)
                            .striped(true)
                            .column(Column::auto().at_least(160.0))
                            .column(Column::auto().at_least(64.0))
                            .column(Column::remainder())
                            .header(20.0, |mut row| {
                                row.col(|ui| {
                                    ui.label(egui::RichText::new("Parameter").small().strong());
                                });
                                row.col(|ui| {
                                    ui.label(egui::RichText::new("Kind").small().strong());
                                });
                                row.col(|ui| {
                                    ui.label(egui::RichText::new("Summary").small().strong());
                                });
                            })
                            .body(|mut body| {
                                for entry in rows {
                                    body.row(22.0, |mut row| {
                                        row.col(|ui| {
                                            ui.label(egui::RichText::new(&entry.title).color(TEXT));
                                        });
                                        row.col(|ui| {
                                            ui.label(
                                                egui::RichText::new(entry.kind)
                                                    .small()
                                                    .color(MUTED),
                                            );
                                        });
                                        row.col(|ui| {
                                            ui.label(
                                                egui::RichText::new(&entry.summary)
                                                    .small()
                                                    .color(MUTED),
                                            );
                                        });
                                    });
                                }
                            });
                    });
                }

                if self
                    .tcm_param_view
                    .entries
                    .iter()
                    .all(|e| e.group == TcmParameterGroup::Other)
                {
                    ui.label(
                        egui::RichText::new(
                            "Load a transmission XDF or rename tables with shift, pressure, or TCC keywords to populate this panel.",
                        )
                        .small()
                        .color(MUTED),
                    );
                }
            });
        });
    }
}

// ---------------------------------------------------------------------------
// Toyota update assistant: record before, check the update file, hand the cable
// to Techstream, verify after. Read-only; Toyota's software performs the update.
// ---------------------------------------------------------------------------

fn check_level_color(level: CheckLevel) -> Color32 {
    match level {
        CheckLevel::Pass => EMERALD,
        CheckLevel::Warn => Color32::from_rgb(255, 193, 7),
        CheckLevel::Fail => Color32::from_rgb(255, 82, 82),
    }
}

fn check_level_mark(level: CheckLevel) -> &'static str {
    match level {
        CheckLevel::Pass => "✔",
        CheckLevel::Warn => "⚠",
        CheckLevel::Fail => "✖",
    }
}

impl WidebandApp {
    /// Identifies the ECU, refreshes battery voltage, and returns a snapshot for `stage`.
    fn capture_update_snapshot(&mut self, stage: &str) -> Option<UpdateSnapshot> {
        let prof = self.profiles[self.selected_profile_idx].clone();
        let allow_demo = self.active_device_virtual;
        let device_name = self.connected_device_label.clone();
        let (result, vbatt) = match self.active_device.as_mut() {
            Some(dev) => {
                let channel = dev.channel_id();
                let result = EcuIdentifier::query(dev.as_mut(), channel, &prof, allow_demo);
                let vbatt = read_pass_thru_vbatt(dev.as_mut(), channel);
                (result, vbatt)
            }
            None => {
                self.log("Update assistant: connect the cable first.");
                return None;
            }
        };
        if vbatt.is_some() {
            self.header_vbatt = vbatt;
        }
        match result {
            Ok(id) => {
                self.log(&id.details);
                let snapshot = UpdateSnapshot::capture(stage, &id, &prof.id, &device_name, self.header_vbatt);
                self.ecu_id = id;
                if let Err(e) = snapshot.append_history(&UpdateSnapshot::history_path()) {
                    self.log(&format!("Update assistant: {}", e));
                }
                Some(snapshot)
            }
            Err(e) => {
                self.log(&format!("ECU identification failed: {}", e));
                None
            }
        }
    }

    fn render_update_assistant(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.heading("Toyota Update Assistant (Techstream / TIS)");
            ui.label(
                egui::RichText::new(
                    "Toyota's own software installs calibration updates. Wideband records the ECU before, checks the update file against your Cal ID, hands the cable to Techstream, and confirms the new Cal ID after. Nothing here writes to the ECU.",
                )
                .small()
                .color(MUTED),
            );
            ui.add_space(6.0);

            // Step 1: record before
            ui.label(egui::RichText::new("1. Record the ECU before the update").strong());
            ui.horizontal_wrapped(|ui| {
                if ui.button("Record ECU (before)").clicked() {
                    if let Some(snapshot) = self.capture_update_snapshot("before") {
                        match snapshot.save(&UpdateSnapshot::default_before_path()) {
                            Ok(()) => self.log(&format!(
                                "Update assistant: before-update record saved to {} (Cal ID {}).",
                                UpdateSnapshot::default_before_path().display(),
                                snapshot.cal_ids.join(", ")
                            )),
                            Err(e) => self.log(&format!("Update assistant: {}", e)),
                        }
                        self.update_before = Some(snapshot);
                        self.update_result = None;
                    }
                }
                match &self.update_before {
                    Some(b) => ui.label(
                        egui::RichText::new(format!(
                            "Recorded {} — Cal ID {}",
                            b.taken_at.get(..19).unwrap_or(&b.taken_at).replace('T', " "),
                            if b.cal_ids.is_empty() { "(none)".to_string() } else { b.cal_ids.join(", ") }
                        ))
                        .small()
                        .color(TEXT),
                    ),
                    None => ui.label(egui::RichText::new("No before-update record yet.").small().color(MUTED)),
                };
            });
            let cable_label = if self.is_connected {
                self.connected_device_label.clone()
            } else {
                String::new()
            };
            if self.is_connected {
                for item in readiness(&self.ecu_id, self.header_vbatt, &cable_label) {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            egui::RichText::new(format!("{} {}", check_level_mark(item.level), item.label))
                                .color(check_level_color(item.level)),
                        );
                        ui.label(egui::RichText::new(&item.detail).small().color(MUTED));
                    });
                }
            }
            ui.add_space(6.0);

            // Step 2: check update file
            ui.label(egui::RichText::new("2. Check the update file").strong());
            ui.horizontal_wrapped(|ui| {
                if ui.button("Check update file (.cuw)…").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Toyota calibration update", &["cuw", "CUW"])
                        .add_filter("All files", &["*"])
                        .pick_file()
                    {
                        match fs::read(&path) {
                            Ok(bytes) => {
                                let cal_ids = if !self.ecu_id.cal_ids.is_empty() {
                                    self.ecu_id.cal_ids.clone()
                                } else {
                                    self.update_before.as_ref().map(|b| b.cal_ids.clone()).unwrap_or_default()
                                };
                                let check = check_update_file(&bytes, &cal_ids);
                                self.update_file_name = path
                                    .file_name()
                                    .map(|n| n.to_string_lossy().to_string())
                                    .unwrap_or_default();
                                self.log(&format!("Update file {}: {}", self.update_file_name, check.summary));
                                self.update_file_check = Some(check);
                            }
                            Err(e) => self.log(&format!("Failed to read update file: {}", e)),
                        }
                    }
                }
                if !self.update_file_name.is_empty() {
                    ui.label(egui::RichText::new(&self.update_file_name).small().color(TEXT));
                }
            });
            if let Some(check) = &self.update_file_check {
                ui.label(
                    egui::RichText::new(format!("{} {}", check_level_mark(check.verdict), check.summary))
                        .color(check_level_color(check.verdict)),
                );
                let payload = match check.payload {
                    PayloadKind::SRecords => "Payload: plain S-records.",
                    PayloadKind::Opaque => "Payload: binary or encrypted. Only Toyota's software can install it.",
                    PayloadKind::None => "Payload: none found after the header.",
                };
                ui.label(egui::RichText::new(payload).small().color(MUTED));
                if !check.header.is_empty() {
                    egui::CollapsingHeader::new("File header").show(ui, |ui| {
                        egui::ScrollArea::vertical().max_height(140.0).show(ui, |ui| {
                            ui.label(egui::RichText::new(&check.header).monospace().small());
                        });
                    });
                }
            }
            ui.add_space(6.0);

            // Step 3: hand off to Techstream
            ui.label(egui::RichText::new("3. Run the update in Techstream").strong());
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(self.is_connected, egui::Button::new("Release cable for Techstream"))
                    .clicked()
                {
                    self.disconnect_cable();
                    self.log(
                        "Cable released for Techstream. Keep a charger in power-supply mode on the battery, run the update in Techstream, close Techstream, then Connect here again.",
                    );
                }
                ui.label(
                    egui::RichText::new("Only one program can use the cable at a time.")
                        .small()
                        .color(MUTED),
                );
            });
            ui.add_space(6.0);

            // Step 4: verify after
            ui.label(egui::RichText::new("4. Verify after the update").strong());
            ui.horizontal_wrapped(|ui| {
                ui.label("New Cal ID from the bulletin:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.update_expected_cal)
                        .hint_text("optional")
                        .desired_width(140.0),
                );
                let can_verify = self.is_connected && self.update_before.is_some();
                if ui
                    .add_enabled(can_verify, egui::Button::new("Verify ECU (after)"))
                    .clicked()
                {
                    if let Some(after) = self.capture_update_snapshot("after") {
                        if let Some(before) = &self.update_before {
                            let result = compare_snapshots(before, &after, &self.update_expected_cal);
                            self.log(&format!("Update verification: {}", result.summary));
                            self.update_result = Some(result);
                        }
                    }
                }
            });
            if let Some(result) = &self.update_result {
                ui.label(
                    egui::RichText::new(format!("{} {}", check_level_mark(result.verdict), result.summary))
                        .color(check_level_color(result.verdict)),
                );
                for (old, new) in &result.changes {
                    ui.label(
                        egui::RichText::new(format!(
                            "{} → {}",
                            if old.is_empty() { "(none)" } else { old },
                            if new.is_empty() { "(none)" } else { new }
                        ))
                        .monospace()
                        .small(),
                    );
                }
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Tab 3: Diagnostics & ECU Flashing
// ---------------------------------------------------------------------------

impl WidebandApp {
    fn render_flash_tab(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.heading(egui::RichText::new("ECU Diagnostics & Laboratory Bench-Flashing").size(16.0).color(Color32::WHITE));
                ui.label("Direct J2534 ISO 14229 / ISO 14230 flashing with seed-key handshake and checksum stamping.");
                ui.separator();

                ui.horizontal_wrapped(|ui| {
                    if ui.button(egui::RichText::new("🔍 Identify ECU").strong()).clicked() {
                        if let Some(dev) = &mut self.active_device {
                            let prof = &self.profiles[self.selected_profile_idx];
                            let allow_demo = self.active_device_virtual;
                            let channel = dev.channel_id();
                            match EcuIdentifier::query(dev.as_mut(), channel, prof, allow_demo) {
                                Ok(id) => {
                                    self.ecu_id = id;
                                    self.log(&format!("ECU Identified: VIN={}, Cal ID={}, CVN={}", self.ecu_id.vin, self.ecu_id.cal_id, self.ecu_id.cvn));
                                }
                                Err(e) => self.log(&format!("ECU identification failed: {}", e)),
                            }
                        } else {
                            self.log("Connect cable first.");
                        }
                    }

                    if ui.button("Read Stock ROM (0x23)").clicked() {
                        self.execute_stock_read();
                    }

                    let prof = &self.profiles[self.selected_profile_idx];
                    let virtual_ecu = self.cable_is_virtual();
                    let flash_blocked = !PlatformSecurity::flash_blockers(&prof.id, virtual_ecu).is_empty();
                    let flash_btn = ui.add_enabled(
                        !flash_blocked,
                        egui::Button::new(
                            egui::RichText::new("⚡ Flash Active Calibration")
                                .strong()
                                .color(Color32::from_rgb(255, 23, 68)),
                        ),
                    );
                    if flash_btn.clicked() {
                        self.flash_confirm_open = true;
                    }
                });

                self.render_security_status_panel(ui);

                if self.flash_confirm_open {
                    let prof = self.profiles[self.selected_profile_idx].clone();
                    let virtual_ecu = self.cable_is_virtual();
                    let blockers = PlatformSecurity::flash_blockers(&prof.id, virtual_ecu);
                    let segments = self
                        .flash_layout
                        .cal_segments(&prof.id)
                        .unwrap_or_default();
                    let mut confirm = false;
                    let mut cancel = false;
                    egui::Window::new("Confirm calibration flash")
                        .collapsible(false)
                        .resizable(false)
                        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                        .show(ctx, |ui| {
                            ui.label(
                                "This writes calibration segments to the connected ECU. Verify vehicle, profile, and security-config before continuing.",
                            );
                            ui.separator();
                            ui.label(format!("Profile: {}", prof.name));
                            for seg in &segments {
                                ui.label(format!(
                                    "  • {} — {} bytes @ 0x{:X}",
                                    seg.label,
                                    seg.size,
                                    seg.address
                                ));
                            }
                            for reason in &blockers {
                                ui.label(
                                    egui::RichText::new(format!("Flash blocked: {}", reason))
                                        .color(Color32::from_rgb(255, 152, 0)),
                                );
                            }
                            if !self.active_device_virtual
                                && !PlatformSecurity::security_key_configured_for_profile(&prof.id)
                                && blockers.is_empty()
                            {
                                ui.label(
                                    egui::RichText::new(
                                        "Production security-config seed-key is not configured for this profile.",
                                    )
                                    .color(Color32::from_rgb(255, 152, 0)),
                                );
                            }
                            ui.horizontal(|ui| {
                                if ui.button(egui::RichText::new("Cancel").strong()).clicked() {
                                    cancel = true;
                                }
                                let write_enabled = blockers.is_empty();
                                if ui
                                    .add_enabled(
                                        write_enabled,
                                        egui::Button::new(
                                            egui::RichText::new("Write calibration to ECU")
                                                .strong()
                                                .color(Color32::from_rgb(255, 23, 68)),
                                        ),
                                    )
                                    .clicked()
                                {
                                    confirm = true;
                                }
                            });
                        });
                    if cancel {
                        self.flash_confirm_open = false;
                    }
                    if confirm && blockers.is_empty() {
                        self.flash_confirm_open = false;
                        self.execute_flash_active();
                    }
                }
                ui.add_space(8.0);
                ui.group(|ui| {
                    ui.heading("Active ECU Metadata");
                    ui.label(format!("Vehicle VIN:       {}", if self.ecu_id.vin.is_empty() { "Not queried" } else { &self.ecu_id.vin }));
                    ui.label(format!("Calibration ID:    {}", if self.ecu_id.cal_id.is_empty() { "Not queried" } else { &self.ecu_id.cal_id }));
                    ui.label(format!("Verification CVN:  {}", if self.ecu_id.cvn.is_empty() { "Not queried" } else { &self.ecu_id.cvn }));
                });

                ui.add_space(8.0);
                self.render_update_assistant(ui);

                ui.add_space(8.0);
                ui.label(format!("Flash Transfer Progress: {:.0}%", self.transfer_progress * 100.0));
                if !self.flash_status_detail.is_empty() {
                    ui.label(egui::RichText::new(&self.flash_status_detail).small().color(MUTED));
                }
                ui.add(egui::ProgressBar::new(self.transfer_progress).animate(self.is_flashing));

                ui.add_space(12.0);
                self.render_diagnostics_panel(ui);

                ui.add_space(8.0);
                ui.heading("Communication & Flash Event Log");
                egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                    for line in &self.comm_log {
                        ui.label(egui::RichText::new(line).monospace().small());
                    }
                });
            });
        });
    }

    fn render_diagnostics_panel(&mut self, ui: &mut egui::Ui) {
        egui::Frame::none()
            .fill(CARD_FILL)
            .stroke(Stroke::new(1.0, STROKE))
            .rounding(egui::Rounding::same(6.0))
            .inner_margin(egui::Margin::symmetric(14.0, 12.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading(
                        egui::RichText::new("Emissions Diagnostics")
                            .size(15.0)
                            .color(ACCENT_CYAN),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new("READ-ONLY")
                                .small()
                                .strong()
                                .color(Color32::from_rgb(140, 155, 175)),
                        );
                    });
                });
                ui.label(
                    egui::RichText::new(
                        "View stored/pending DTCs, freeze frames, and OBD readiness. Clear-DTC and MIL-defeat paths are not exposed.",
                    )
                    .small()
                    .color(MUTED),
                );
                ui.add_space(6.0);

                ui.horizontal(|ui| {
                    if ui
                        .button(egui::RichText::new("↻ Refresh DTCs & Readiness").strong())
                        .clicked()
                    {
                        self.refresh_diagnostics();
                    }
                    let mil_color = if self.diag_snapshot.mil_on {
                        Color32::from_rgb(255, 82, 82)
                    } else {
                        EMERALD
                    };
                    let mil_text = if self.diag_snapshot.mil_on {
                        "MIL: ON"
                    } else {
                        "MIL: OFF"
                    };
                    ui.label(egui::RichText::new(mil_text).strong().color(mil_color));
                });

                if !self.diag_last_error.is_empty() {
                    ui.label(
                        egui::RichText::new(&self.diag_last_error)
                            .small()
                            .color(Color32::from_rgb(255, 152, 0)),
                    );
                }

                ui.add_space(8.0);
                ui.columns(2, |columns| {
                    columns[0].heading(egui::RichText::new("Stored DTCs").size(13.0).color(MUTED));
                    columns[1].heading(egui::RichText::new("Pending DTCs").size(13.0).color(MUTED));

                    Self::render_dtc_table(&mut columns[0], &self.diag_snapshot.stored_dtcs, &mut self.diag_selected_dtc);
                    Self::render_dtc_table_pending(&mut columns[1], &self.diag_snapshot.pending_dtcs);
                });

                ui.add_space(10.0);
                ui.heading(egui::RichText::new("OBD Readiness Monitors").size(13.0).color(MUTED));
                egui::Grid::new("readiness_grid")
                    .num_columns(3)
                    .spacing([12.0, 4.0])
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new("Monitor").small().strong());
                        ui.label(egui::RichText::new("Supported").small().strong());
                        ui.label(egui::RichText::new("Complete").small().strong());
                        ui.end_row();
                        for mon in &self.diag_snapshot.readiness.monitors {
                            ui.label(&mon.name);
                            ui.label(yes_no(mon.supported));
                            let complete_color = if mon.complete { EMERALD } else { Color32::from_rgb(255, 193, 7) };
                            ui.label(egui::RichText::new(yes_no(mon.complete)).color(complete_color));
                            ui.end_row();
                        }
                    });

                ui.add_space(10.0);
                ui.heading(egui::RichText::new("Freeze Frame").size(13.0).color(MUTED));
                let ff = self.selected_freeze_frame();
                if let Some(frame) = ff {
                    ui.label(
                        egui::RichText::new(format!(
                            "DTC {} — record 0x{:02X}",
                            frame.dtc_code,
                            frame.record_number
                        ))
                        .monospace()
                        .color(TEXT),
                    );
                    for pid in &frame.pid_values {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(format!("PID 0x{:02X}", pid.pid)).small().color(MUTED));
                            ui.label(egui::RichText::new(&pid.label).color(TEXT));
                            ui.label(egui::RichText::new(&pid.value).monospace().strong().color(ACCENT_CYAN));
                        });
                    }
                } else {
                    ui.label(egui::RichText::new("No freeze frame loaded — refresh diagnostics or select a stored DTC.").small().color(MUTED));
                }
            });
    }

    fn render_dtc_table(ui: &mut egui::Ui, dtcs: &[diagnostics::DiagnosticTroubleCode], selected: &mut usize) {
        if dtcs.is_empty() {
            ui.label(egui::RichText::new("None").small().color(MUTED));
            return;
        }
        TableBuilder::new(ui)
            .striped(true)
            .column(Column::auto().at_least(72.0))
            .column(Column::remainder())
            .column(Column::auto().at_least(72.0))
            .header(20.0, |mut row| {
                row.col(|ui| {
                    ui.label(egui::RichText::new("Code").small().strong());
                });
                row.col(|ui| {
                    ui.label(egui::RichText::new("Description").small().strong());
                });
                row.col(|ui| {
                    ui.label(egui::RichText::new("Status").small().strong());
                });
            })
            .body(|mut body| {
                for (idx, dtc) in dtcs.iter().enumerate() {
                    body.row(22.0, |mut row| {
                        row.col(|ui| {
                            let sel = *selected == idx;
                            if ui.selectable_label(sel, egui::RichText::new(&dtc.code).monospace().strong()).clicked() {
                                *selected = idx;
                            }
                        });
                        row.col(|ui| {
                            ui.label(egui::RichText::new(&dtc.description).small());
                        });
                        row.col(|ui| {
                            let c = if dtc.status == DtcStatus::Pending {
                                Color32::from_rgb(255, 193, 7)
                            } else {
                                ACCENT_CYAN
                            };
                            ui.label(egui::RichText::new(status_label(dtc.status)).small().color(c));
                        });
                    });
                }
            });
    }

    fn render_dtc_table_pending(ui: &mut egui::Ui, dtcs: &[diagnostics::DiagnosticTroubleCode]) {
        if dtcs.is_empty() {
            ui.label(egui::RichText::new("None").small().color(MUTED));
            return;
        }
        for dtc in dtcs {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(&dtc.code).monospace().strong().color(ACCENT_CYAN));
                ui.label(egui::RichText::new(&dtc.description).small());
            });
        }
    }

    fn selected_freeze_frame(&self) -> Option<&diagnostics::FreezeFrame> {
        if self.diag_snapshot.freeze_frames.is_empty() {
            return None;
        }
        if let Some(dtc) = self.diag_snapshot.stored_dtcs.get(self.diag_selected_dtc) {
            return self
                .diag_snapshot
                .freeze_frames
                .iter()
                .find(|f| f.dtc_code == dtc.code);
        }
        self.diag_snapshot.freeze_frames.first()
    }

    pub(crate) fn refresh_diagnostics(&mut self) {
        self.diag_last_error.clear();
        if self.active_device.is_none() {
            self.diag_last_error = "Connect cable first.".to_string();
            return;
        }
        let prof = self.profiles[self.selected_profile_idx].clone();
        if let Some(dev) = &mut self.active_device {
            let channel = dev.channel_id();
            match diagnostics::DiagnosticsReader::read(dev.as_mut(), channel, &prof) {
                Ok(snap) => {
                    self.diag_snapshot = snap;
                    self.diag_selected_dtc = 0;
                    let n = self.diag_snapshot.stored_dtcs.len();
                    let p = self.diag_snapshot.pending_dtcs.len();
                    self.log(&format!(
                        "Diagnostics: {} stored, {} pending DTC(s); MIL {}",
                        n,
                        p,
                        if self.diag_snapshot.mil_on { "ON" } else { "OFF" }
                    ));
                }
                Err(e) => {
                    self.log(&format!("Diagnostics read failed: {}", e));
                    self.diag_last_error = e;
                }
            }
        }
    }

    fn execute_stock_read(&mut self) {
        if self.active_device.is_none() {
            self.log("Stock read blocked: Connect cable first.");
            return;
        }
        let prof = self.profiles[self.selected_profile_idx].clone();
        self.log(&format!("Starting multi-block stock read for {}...", prof.name));
        self.is_flashing = true;
        self.transfer_progress = 0.0;
        if let Some(dev) = &mut self.active_device {
            let channel = dev.channel_id();
            let label = dev.device_label();
            let mut read_progress = 0.0f32;
            let mut read_detail = String::new();
            let read_res = FlashJob::read_stock(
                dev.as_mut(),
                channel,
                &prof,
                &self.flash_layout,
                &label,
                &self.flash_audit,
                |done, total| {
                    read_progress = done as f32 / total.max(1) as f32;
                    read_detail = format!("Stock read: {} / {} bytes", done, total);
                },
            );
            self.transfer_progress = read_progress;
            self.flash_status_detail = read_detail;
            match read_res {
                Ok(stock) => {
                    self.active_rom = stock.bytes;
                    self.rom_from_ecu = true;
                    self.rom_origin = stock.origin;
                    self.xdf.bind_from_rom(&self.active_rom, self.rom_origin);
                    self.refresh_stock_baseline();
                    self.refresh_checksum_badge();
                    self.transfer_progress = 1.0;
                    self.log(&format!(
                        "Stock read complete: {} bytes @ origin 0x{:X} (audit JSONL appended)",
                        self.active_rom.len(),
                        self.rom_origin
                    ));
                }
                Err(e) => {
                    let _ = self.flash_audit.record_write(
                        "stock_read",
                        &prof.id,
                        &label,
                        0,
                        0,
                        None,
                        "error",
                        &e,
                    );
                    self.log(&format!("Stock read failed: {}", e));
                }
            }
        }
        self.is_flashing = false;
    }

    fn execute_flash_active(&mut self) {
        if self.active_device.is_none() {
            self.log("Flashing blocked: Connect cable first.");
            return;
        }

        let prof = self.profiles[self.selected_profile_idx].clone();
        let virtual_ecu = self.cable_is_virtual();
        if let Err(e) = PlatformSecurity::assert_flash_allowed(&prof.id, virtual_ecu) {
            self.log(&format!("Flash blocked: {}", e));
            return;
        }

        self.log("Stamping and verifying checksum before cal-only flash...");
        self.xdf.write_all_tables_to_rom(&mut self.active_rom, self.rom_origin);
        self.xdf.write_all_constants_to_rom(&mut self.active_rom, self.rom_origin);
        for flag in &self.xdf.flags {
            self.xdf.write_flag_to_rom(&mut self.active_rom, self.rom_origin, flag);
        }
        if self.checksum_status == CHECKSUM_NOT_CONFIGURED {
            self.log(&format!(
                "Flash blocked: {} — configure checksum ranges in security-config.json.",
                CHECKSUM_NOT_CONFIGURED
            ));
            return;
        }
        let prepared = match PlatformSecurity::ensure_flash_checksum(
            &prof.checksum_profile_id,
            &prof.name,
            &self.active_rom,
            self.rom_origin,
        ) {
            Ok(img) => img,
            Err(e) => {
                self.log(&format!("Flash blocked: {}", e));
                return;
            }
        };
        if let Err(e) = self.run_profile_plugin_hooks(&prof, &prepared, self.rom_origin) {
            self.log(&format!("Flash blocked (plugin hook): {}", e));
            return;
        }

        self.log(&format!("Starting layout-driven cal flash for {}...", prof.name));
        self.is_flashing = true;
        self.transfer_progress = 0.0;
        self.flash_status_detail.clear();

        if let Some(dev) = &mut self.active_device {
            let channel = dev.channel_id();
            let label = dev.device_label();
            let mut last_frac = 0.0f32;
            let mut last_detail = String::new();
            let require_production_security = !self.active_device_virtual;
            let flash_res = FlashJob::flash_cal_segments(
                dev.as_mut(),
                channel,
                &prof,
                &self.flash_layout,
                &prepared,
                self.rom_origin,
                &label,
                &self.flash_audit,
                |frac, detail| {
                    last_frac = frac;
                    last_detail = detail.to_string();
                },
                require_production_security,
            );
            self.transfer_progress = last_frac;
            self.flash_status_detail = last_detail;

            match flash_res {
                Ok(()) => {
                    self.transfer_progress = 1.0;
                    self.is_flashing = false;
                    self.log("Cal segment flash completed (audit JSONL appended).");
                }
                Err(e) => {
                    self.is_flashing = false;
                    let _ = self.flash_audit.record_write(
                        "cal_flash",
                        &prof.id,
                        &label,
                        0,
                        0,
                        None,
                        "error",
                        &e,
                    );
                    self.log(&format!("Flash failed: {}", e));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tab 4: Switchboard (<XDFFLAG> Bit Switches)
// ---------------------------------------------------------------------------

impl WidebandApp {
    fn render_switchboard_tab(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading(egui::RichText::new("ECU Switchboard (<XDFFLAG> Bitmask Switches)").size(16.0).color(Color32::WHITE));
            ui.label("Toggle ECU feature switches directly. Changes are committed to the active ROM image.");
            ui.separator();

            egui::ScrollArea::vertical().show(ui, |ui| {
                for idx in 0..self.xdf.flags.len() {
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            let mut checked = self.xdf.flags[idx].is_set;
                            if ui.checkbox(&mut checked, "").changed() {
                                self.xdf.flags[idx].is_set = checked;
                                let flag = self.xdf.flags[idx].clone();
                                self.xdf.write_flag_to_rom(&mut self.active_rom, self.rom_origin, &flag);
                                self.refresh_checksum_badge();
                            }

                            let flag = &self.xdf.flags[idx];
                            let stock_set = self.stock_flags.get(idx).copied().unwrap_or(flag.is_set);
                            let flag_changed = flag.is_set != stock_set;
                            let badge_color = if flag.is_set {
                                Color32::from_rgb(0, 230, 118)
                            } else {
                                Color32::from_rgb(140, 150, 165)
                            };
                            let status_lbl = if flag.is_set { "[ENABLED]" } else { "[DISABLED]" };
                            ui.label(egui::RichText::new(status_lbl).strong().color(badge_color));

                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(&flag.title).strong().color(Color32::WHITE));
                                    if flag_changed {
                                        ui.label(
                                            egui::RichText::new("(changed from stock)")
                                                .small()
                                                .strong()
                                                .color(Color32::from_rgb(255, 214, 0)),
                                        );
                                    }
                                });
                                ui.label(egui::RichText::new(&flag.description).small().color(Color32::GRAY));
                                ui.label(egui::RichText::new(format!("Address: 0x{:X} | Mask: 0x{:02X}", flag.address, flag.mask)).monospace().small());
                            });
                        });
                    });
                    ui.add_space(4.0);
                }
            });
        });
    }
}

// ---------------------------------------------------------------------------
// Log-based VE / fuel-table correction panel
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum VeLogSource {
    /// Frames captured with Record on the Telemetry tab.
    Recording,
    /// The rolling live buffer (most recent samples).
    LiveBuffer,
    /// The primary log imported on the Telemetry tab.
    ImportedLog,
}

pub struct VeCorrectionUi {
    pub source: VeLogSource,
    /// `None` = constant target lambda; `Some(idx)` = commanded-lambda table index.
    pub target_table: Option<usize>,
    pub target_const: f64,
    pub settings: CorrectionSettings,
    pub result: Option<(usize, CorrectionResult)>,
    pub show_overlay: bool,
    pub message: String,
}

impl Default for VeCorrectionUi {
    fn default() -> Self {
        Self {
            source: VeLogSource::Recording,
            target_table: None,
            target_const: 1.0,
            settings: CorrectionSettings::default(),
            result: None,
            show_overlay: true,
            message: String::new(),
        }
    }
}

impl VeCorrectionUi {
    pub fn overlay_for(&self, table_idx: usize) -> Option<&CorrectionResult> {
        match &self.result {
            Some((idx, res)) if *idx == table_idx && self.show_overlay => Some(res),
            _ => None,
        }
    }
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// Heuristic: tables where a larger value means more fuel (correction direction is valid).
fn looks_like_fuel_table(t: &crate::definitions::XdfTable) -> bool {
    let s = format!("{} {} {}", t.id, t.title, t.z_units).to_ascii_lowercase();
    ["ve", "fuel", "inj", "airflow", "volumetric"].iter().any(|k| s.contains(k))
        && !s.contains("lambda")
        && !s.contains("afr")
}

fn looks_like_lambda_target(t: &crate::definitions::XdfTable) -> bool {
    let s = format!("{} {}", t.id, t.title).to_ascii_lowercase();
    (s.contains("lambda") || s.contains("afr") || s.contains("eq ratio")) && t.values.len() >= 2
}

impl WidebandApp {
    fn render_ve_correction_panel(&mut self, ui: &mut egui::Ui) {
        let table_idx = self.selected_table_idx;
        let table_title = self.xdf.tables[table_idx].title.clone();
        let fuel_like = looks_like_fuel_table(&self.xdf.tables[table_idx]);
        let (rec_n, live_n) = {
            let buf = self.telemetry.lock().unwrap();
            (buf.record_frames.len(), buf.frames.len())
        };
        let imported = self.telemetry_playback.primary.as_ref().map(|l| (l.name.clone(), l.frames.len()));
        let lambda_tables: Vec<(usize, String)> = self
            .xdf
            .tables
            .iter()
            .enumerate()
            .filter(|(i, t)| *i != table_idx && looks_like_lambda_target(t))
            .map(|(i, t)| (i, t.title.clone()))
            .collect();

        egui::Frame::group(ui.style()).show(ui, |ui| {
            if !fuel_like {
                ui.label(
                    egui::RichText::new(format!(
                        "\u{26A0} \"{}\" doesn't look like a VE / fuel table. Corrections assume a larger value = more fuel.",
                        table_title
                    ))
                    .small()
                    .color(Color32::from_rgb(255, 152, 0)),
                );
            }
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new("Log:").color(MUTED));
                let src_label = |s: VeLogSource| match s {
                    VeLogSource::Recording => format!("Recorded session ({} frames)", rec_n),
                    VeLogSource::LiveBuffer => format!("Live buffer ({} frames)", live_n),
                    VeLogSource::ImportedLog => match &imported {
                        Some((n, f)) => format!("Imported: {} ({} frames)", n, f),
                        None => "Imported log (none loaded)".to_string(),
                    },
                };
                egui::ComboBox::from_id_salt("ve_src")
                    .selected_text(src_label(self.ve_corr.source))
                    .show_ui(ui, |ui| {
                        for s in [VeLogSource::Recording, VeLogSource::LiveBuffer, VeLogSource::ImportedLog] {
                            ui.selectable_value(&mut self.ve_corr.source, s, src_label(s));
                        }
                    });

                ui.label(egui::RichText::new("Target \u{03BB}:").color(MUTED));
                let target_text = match self.ve_corr.target_table {
                    Some(i) => self.xdf.tables.get(i).map(|t| t.title.clone()).unwrap_or_default(),
                    None => "Constant".to_string(),
                };
                egui::ComboBox::from_id_salt("ve_target")
                    .selected_text(target_text)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.ve_corr.target_table, None, "Constant");
                        for (i, title) in &lambda_tables {
                            ui.selectable_value(&mut self.ve_corr.target_table, Some(*i), title);
                        }
                    });
                if self.ve_corr.target_table.is_none() {
                    ui.add(egui::DragValue::new(&mut self.ve_corr.target_const).speed(0.005).range(0.6..=1.5));
                }
                if ui.button("Analyze log").clicked() {
                    self.run_ve_analysis();
                }
            });

            egui::CollapsingHeader::new(egui::RichText::new("Filters").small().color(MUTED))
                .id_salt("ve_filters")
                .show(ui, |ui| {
                    let st = &mut self.ve_corr.settings;
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Min RPM");
                        ui.add(egui::DragValue::new(&mut st.min_rpm).speed(10.0).range(0.0..=3000.0));
                        ui.label("Min ECT \u{00B0}C");
                        ui.add(egui::DragValue::new(&mut st.min_ect_c).speed(1.0).range(-40.0..=110.0));
                        ui.label("\u{03BB} window");
                        ui.add(egui::DragValue::new(&mut st.lambda_min).speed(0.01).range(0.5..=1.0));
                        ui.add(egui::DragValue::new(&mut st.lambda_max).speed(0.01).range(1.0..=2.0));
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Max TPS rate %/s");
                        ui.add(egui::DragValue::new(&mut st.max_tps_rate_pct_s).speed(1.0).range(1.0..=500.0));
                        ui.label("Sensor delay s");
                        ui.add(egui::DragValue::new(&mut st.lambda_delay_s).speed(0.01).range(0.0..=1.0));
                        ui.label("Min samples/cell");
                        ui.add(egui::DragValue::new(&mut st.min_weight).speed(0.5).range(1.0..=200.0));
                        ui.label("Max change %");
                        ui.add(egui::DragValue::new(&mut st.max_change_pct).speed(0.5).range(1.0..=50.0));
                    });
                });

            if !self.ve_corr.message.is_empty() {
                ui.label(egui::RichText::new(&self.ve_corr.message).small().color(MUTED));
            }

            let has_result = matches!(&self.ve_corr.result, Some((i, _)) if *i == table_idx);
            if has_result {
                ui.horizontal_wrapped(|ui| {
                    ui.checkbox(&mut self.ve_corr.show_overlay, "Show suggested % on grid");
                    if ui.button("Select cells with data").clicked() {
                        if let Some((_, res)) = &self.ve_corr.result {
                            if let Some((r0, r1, c0, c1)) = res.data_bounds() {
                                self.selection_anchor = Some((r0, c0));
                                self.active_cell = (r1, c1);
                            }
                        }
                    }
                    if ui
                        .button("Apply to selection")
                        .on_hover_text("Multiply each selected cell that has data by its suggested factor (undoable)")
                        .clicked()
                    {
                        self.apply_ve_correction();
                    }
                    if ui.button("Clear").clicked() {
                        self.ve_corr.result = None;
                        self.ve_corr.message.clear();
                    }
                });
            }
        });
    }

    fn run_ve_analysis(&mut self) {
        let table_idx = self.selected_table_idx;
        let mut settings = self.ve_corr.settings.clone();
        settings.target = match self.ve_corr.target_table.and_then(|i| self.xdf.tables.get(i)) {
            Some(t) => TargetLambda::Table(t.clone()),
            None => TargetLambda::Constant(self.ve_corr.target_const),
        };
        let table = self.xdf.tables[table_idx].clone();
        let result = self.with_log(self.ve_corr.source, |src| ve_correction::analyze(&table, src, &settings));
        match result {
            None => {
                self.ve_corr.result = None;
                self.ve_corr.message =
                    "No frames in that log. Record a drive on the Telemetry tab, or import a CSV log.".to_string();
            }
            Some(res) => {
                let r = &res.rejected;
                self.ve_corr.message = format!(
                    "Used {} samples; rejected {} (low rpm {}, cold {}, \u{03BB} out of window {}, transient {}, off table {}, missing channels {}). \
                     {} cells have suggestions. Fuel trims {}.",
                    res.used,
                    r.total(),
                    r.low_rpm,
                    r.cold,
                    r.lambda_out_of_window,
                    r.transient,
                    r.off_table,
                    r.missing_channels,
                    res.cells_with_data(),
                    if res.used_trims { "included" } else { "not logged (open-loop lambda only)" },
                );
                self.ve_corr.show_overlay = true;
                self.hist.show_overlay = false;
                self.ve_corr.result = Some((table_idx, res));
            }
        }
    }

    fn apply_ve_correction(&mut self) {
        let Some((idx, res)) = self.ve_corr.result.clone() else {
            return;
        };
        if idx != self.selected_table_idx {
            return;
        }
        let mut changed = 0;
        self.edit_selected_table("log fuel correction", |v, sel| {
            changed = ve_correction::apply(v, &res, sel.bounds());
        });
        if changed == 0 {
            self.ve_corr.message = "No selected cells have enough data. Use \"Select cells with data\" first.".to_string();
            return;
        }
        // The suggestions were relative to the old values; drop them so they can't be applied twice.
        self.ve_corr.result = None;
        self.ve_corr.message = format!(
            "Applied to {} cells (Ctrl+Z to undo). Log again with the new table to refine.",
            changed
        );
        self.log(&format!("Log fuel correction applied to {} cells of {}", changed, self.xdf.tables[idx].title));
    }
}

impl WidebandApp {
    /// HP Tuners-style grid keyboard: type a value + Enter, + - * / step ops, arrows /
    /// Shift+arrows, Ctrl+C / Ctrl+V (Excel TSV), Ctrl+A, Ctrl+Z / Ctrl+Y, Esc.
    fn handle_grid_keyboard(&mut self, ui: &mut egui::Ui) {
        if self.xdf.tables.is_empty() || ui.ctx().wants_keyboard_input() {
            return;
        }
        let (rows, cols) = {
            let t = &self.xdf.tables[self.selected_table_idx];
            (t.values.len(), t.values.first().map_or(0, |r| r.len()))
        };
        let events = ui.input(|i| i.events.clone());
        let mut changed_entry = false;
        for ev in events {
            match ev {
                egui::Event::Copy | egui::Event::Cut => {
                    let tsv = table_edit::to_tsv(&self.xdf.tables[self.selected_table_idx].values, &self.selection());
                    if !tsv.is_empty() {
                        let n = self.selection().count();
                        ui.ctx().copy_text(tsv);
                        self.log(&format!("Copied {} cell(s)", n));
                    }
                }
                egui::Event::Paste(text) => match table_edit::parse_block(&text) {
                    Ok(block) => {
                        let mut wrote = None;
                        let mode = self.paste_mode;
                        self.edit_selected_table(&format!("paste ({})", mode.label()), |v, sel| {
                            wrote = table_edit::paste_block(v, sel, &block, mode)
                        });
                        if let Some(w) = wrote {
                            self.selection_anchor = Some(w.anchor);
                            self.active_cell = w.cursor;
                        }
                    }
                    Err(e) => self.log(&format!("Paste refused: {}", e)),
                },
                egui::Event::Text(t) => {
                    for ch in t.chars() {
                        match (&mut self.cell_entry, ch) {
                            (Some(buf), c) if c.is_ascii_digit() || c == '.' || ((c == '-' || c == 'e') && !buf.contains(c)) => {
                                buf.push(c);
                                changed_entry = true;
                            }
                            (None, c) if c.is_ascii_digit() || c == '.' => {
                                self.cell_entry = Some(c.to_string());
                                changed_entry = true;
                            }
                            (None, '=') => {
                                self.cell_entry = Some(String::new());
                                changed_entry = true;
                            }
                            (None, '+') => self.run_table_math(MathOp::Add(self.math_amount)),
                            (None, '-') => self.run_table_math(MathOp::Sub(self.math_amount)),
                            (None, '*') => self.run_table_math(MathOp::Percent(self.math_amount)),
                            (None, '/') => self.run_table_math(MathOp::Percent(-self.math_amount)),
                            _ => {}
                        }
                    }
                }
                egui::Event::Key { key, pressed: true, modifiers, .. } => {
                    let cmd = modifiers.command;
                    match key {
                        egui::Key::Z if cmd && !modifiers.shift => self.undo_table_edit(),
                        egui::Key::Z if cmd && modifiers.shift => self.redo_table_edit(),
                        egui::Key::Y if cmd => self.redo_table_edit(),
                        egui::Key::A if cmd => {
                            self.selection_anchor = Some((0, 0));
                            self.active_cell = (rows.saturating_sub(1), cols.saturating_sub(1));
                        }
                        egui::Key::Enter => {
                            if let Some(buf) = self.cell_entry.take() {
                                match buf.trim().parse::<f64>() {
                                    Ok(v) => self.run_table_math(MathOp::Set(v)),
                                    Err(_) if buf.trim().is_empty() => {}
                                    Err(_) => self.log(&format!("\"{}\" is not a number", buf)),
                                }
                                changed_entry = true;
                            }
                        }
                        egui::Key::Escape => {
                            if self.cell_entry.take().is_none() {
                                self.selection_anchor = None;
                            }
                            changed_entry = true;
                        }
                        egui::Key::Backspace => {
                            if let Some(buf) = &mut self.cell_entry {
                                buf.pop();
                                changed_entry = true;
                            }
                        }
                        egui::Key::ArrowUp | egui::Key::ArrowDown | egui::Key::ArrowLeft | egui::Key::ArrowRight if !cmd => {
                            let (dr, dc) = match key {
                                egui::Key::ArrowUp => (-1, 0),
                                egui::Key::ArrowDown => (1, 0),
                                egui::Key::ArrowLeft => (0, -1),
                                _ => (0, 1),
                            };
                            let moved = table_edit::move_cursor(&self.selection(), dr, dc, modifiers.shift, rows, cols);
                            self.selection_anchor = if moved.is_single() { None } else { Some(moved.anchor) };
                            self.active_cell = moved.cursor;
                            self.cell_entry = None;
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        if changed_entry {
            ui.ctx().request_repaint();
        }
    }
}

// ---------------------------------------------------------------------------
// Scanner histograms
// ---------------------------------------------------------------------------

pub struct HistogramUi {
    pub source: VeLogSource,
    pub value_channel: String,
    pub x_channel: String,
    pub y_channel: String,
    pub aggregate: crate::histogram::Aggregate,
    pub filter_on: bool,
    pub filter: crate::histogram::Filter,
    pub min_hits: u32,
    pub result: Option<crate::histogram::Histogram>,
    pub show_overlay: bool,
    pub message: String,
}

impl Default for HistogramUi {
    fn default() -> Self {
        Self {
            source: VeLogSource::Recording,
            value_channel: "lambda".into(),
            x_channel: "rpm".into(),
            y_channel: "map".into(),
            aggregate: crate::histogram::Aggregate::Mean,
            filter_on: false,
            filter: crate::histogram::Filter { channel: "ect".into(), min: 70.0, max: 130.0 },
            min_hits: 3,
            result: None,
            show_overlay: true,
            message: String::new(),
        }
    }
}

impl HistogramUi {
    /// Histogram + value range when it was built for `table` (same id and shape) and is shown.
    fn overlay_for(&self, table: &crate::definitions::XdfTable) -> Option<(crate::histogram::Histogram, f64, f64)> {
        let h = self.result.as_ref()?;
        let shape_ok = h.rows() == table.values.len() && h.cols() == table.values.first().map_or(0, |r| r.len());
        if !self.show_overlay || h.table_id != table.id || !shape_ok {
            return None;
        }
        let (lo, hi) = h.range().unwrap_or((0.0, 1.0));
        Some((h.clone(), lo, hi))
    }
}

fn histogram_cell_text(v: f64) -> String {
    if v.abs() >= 100.0 {
        format!("{:.0}", v)
    } else if v.abs() >= 10.0 {
        format!("{:.1}", v)
    } else {
        format!("{:.3}", v)
    }
}

impl WidebandApp {
    /// Run `f` over the frames of a log source. `None` when that source has no frames.
    fn with_log<R>(&self, source: VeLogSource, f: impl FnOnce(&dyn ve_correction::SampleSource) -> R) -> Option<R> {
        let buf = self.telemetry.lock().unwrap();
        let live: Vec<crate::telemetry::TelemetryFrame>;
        let frames: &[crate::telemetry::TelemetryFrame] = match source {
            VeLogSource::Recording => &buf.record_frames,
            VeLogSource::LiveBuffer => {
                live = buf.frames.iter().cloned().collect();
                &live
            }
            VeLogSource::ImportedLog => match &self.telemetry_playback.primary {
                Some(log) => &log.frames,
                None => &[],
            },
        };
        if frames.is_empty() {
            return None;
        }
        let src = FrameSource::new(&buf.id_index, frames);
        Some(f(&src))
    }

    fn render_histogram_panel(&mut self, ui: &mut egui::Ui) {
        let table_idx = self.selected_table_idx;
        let channels: Vec<(String, String)> = {
            let buf = self.telemetry.lock().unwrap();
            buf.channels.iter().map(|c| (c.id.clone(), format!("{} ({})", c.label, c.id))).collect()
        };
        let (rec_n, live_n) = {
            let buf = self.telemetry.lock().unwrap();
            (buf.record_frames.len(), buf.frames.len())
        };
        let imported = self.telemetry_playback.primary.as_ref().map(|l| (l.name.clone(), l.frames.len()));

        egui::Frame::group(ui.style()).show(ui, |ui| {
                let channel_combo = |ui: &mut egui::Ui, id: &str, value: &mut String| {
                    let shown = channels.iter().find(|(c, _)| c == value).map(|(_, l)| l.clone()).unwrap_or_else(|| value.clone());
                    egui::ComboBox::from_id_salt(id).selected_text(shown).width(170.0).show_ui(ui, |ui| {
                        for (cid, label) in &channels {
                            ui.selectable_value(value, cid.clone(), label);
                        }
                    });
                };
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new("Log:").color(MUTED));
                    let src_label = |s: VeLogSource| match s {
                        VeLogSource::Recording => format!("Recorded ({} frames)", rec_n),
                        VeLogSource::LiveBuffer => format!("Live buffer ({})", live_n),
                        VeLogSource::ImportedLog => match &imported {
                            Some((n, f)) => format!("Imported: {} ({})", n, f),
                            None => "Imported (none)".to_string(),
                        },
                    };
                    egui::ComboBox::from_id_salt("hist_src").selected_text(src_label(self.hist.source)).show_ui(ui, |ui| {
                        for s in [VeLogSource::Recording, VeLogSource::LiveBuffer, VeLogSource::ImportedLog] {
                            ui.selectable_value(&mut self.hist.source, s, src_label(s));
                        }
                    });
                    egui::ComboBox::from_id_salt("hist_agg").selected_text(self.hist.aggregate.label()).show_ui(ui, |ui| {
                        for a in crate::histogram::Aggregate::ALL {
                            ui.selectable_value(&mut self.hist.aggregate, a, a.label());
                        }
                    });
                    ui.label(egui::RichText::new("of").color(MUTED));
                    channel_combo(ui, "hist_val", &mut self.hist.value_channel);
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new("Columns (X):").color(MUTED));
                    channel_combo(ui, "hist_x", &mut self.hist.x_channel);
                    ui.label(egui::RichText::new("Rows (Y):").color(MUTED));
                    channel_combo(ui, "hist_y", &mut self.hist.y_channel);
                    ui.label(egui::RichText::new("Min hits").color(MUTED));
                    ui.add(egui::DragValue::new(&mut self.hist.min_hits).range(1..=1000));
                });
                ui.horizontal_wrapped(|ui| {
                    ui.checkbox(&mut self.hist.filter_on, "Only when");
                    channel_combo(ui, "hist_filter", &mut self.hist.filter.channel);
                    ui.label("between");
                    ui.add(egui::DragValue::new(&mut self.hist.filter.min).speed(0.5));
                    ui.label("and");
                    ui.add(egui::DragValue::new(&mut self.hist.filter.max).speed(0.5));
                    if ui.button("Use table axes").on_hover_text("Pick X/Y channels from this table's axis units").clicked() {
                        let t = &self.xdf.tables[table_idx];
                        if let Some(x) = live_trace::channel_for_axis_units(&t.x_axis.units) {
                            self.hist.x_channel = x.to_string();
                        }
                        if let Some(y) = live_trace::channel_for_axis_units(&t.y_axis.units) {
                            self.hist.y_channel = y.to_string();
                        }
                    }
                    if ui.button("Build histogram").clicked() {
                        self.build_histogram();
                    }
                });
                if !self.hist.message.is_empty() {
                    ui.label(egui::RichText::new(&self.hist.message).small().color(MUTED));
                }
                let built_here = self
                    .hist
                    .result
                    .as_ref()
                    .map_or(false, |h| h.table_id == self.xdf.tables[table_idx].id);
                if built_here {
                    ui.horizontal_wrapped(|ui| {
                        if ui.checkbox(&mut self.hist.show_overlay, "Show on grid").changed() && self.hist.show_overlay {
                            self.ve_corr.show_overlay = false;
                        }
                        if ui
                            .button("Copy histogram")
                            .on_hover_text("Tab-separated; empty bins stay empty. Then pick a Ctrl+V mode and paste into any table.")
                            .clicked()
                        {
                            if let Some(h) = &self.hist.result {
                                ui.ctx().copy_text(crate::table_edit::grid_to_tsv(&h.grid()));
                                // Paste lands at the cursor: put it on the first cell so the bins line up.
                                self.selection_anchor = None;
                                self.active_cell = (0, 0);
                                self.hist.message = "Copied. Select the target table's first cell, choose the Ctrl+V mode, and paste.".into();
                            }
                        }
                        if ui.button("Clear").clicked() {
                            self.hist.result = None;
                            self.hist.message.clear();
                        }
                    });
                }
            });
    }

    fn build_histogram(&mut self) {
        let table = self.xdf.tables[self.selected_table_idx].clone();
        let spec = crate::histogram::HistogramSpec {
            value_channel: self.hist.value_channel.clone(),
            x_channel: self.hist.x_channel.clone(),
            y_channel: self.hist.y_channel.clone(),
            aggregate: self.hist.aggregate,
            filter: self.hist.filter_on.then(|| self.hist.filter.clone()),
            min_hits: self.hist.min_hits,
        };
        let res = self.with_log(self.hist.source, |src| {
            crate::histogram::build(&table.id, &table.x_axis.labels, &table.y_axis.labels, src, &spec)
        });
        match res {
            None => {
                self.hist.result = None;
                self.hist.message = "No frames in that log. Record on the Telemetry tab or import a CSV.".into();
            }
            Some(h) => {
                let filled = h.grid().iter().flatten().filter(|c| c.is_some()).count();
                self.hist.message = format!(
                    "{} samples binned into {} cells; {} filtered out, {} outside the table axes, {} missing a channel.",
                    h.used, filled, h.filtered, h.off_axes, h.missing
                );
                self.hist.show_overlay = true;
                self.ve_corr.show_overlay = false;
                self.hist.result = Some(h);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tune compare
// ---------------------------------------------------------------------------

impl WidebandApp {
    /// Tables the grid's change markers measure against (stock as opened, or the compare file).
    fn baseline_tables(&self) -> &[Vec<Vec<f64>>] {
        match (&self.compare, self.diff_vs_compare) {
            (Some(c), true) if c.tables.len() == self.xdf.tables.len() => &c.tables,
            _ => &self.stock_tables,
        }
    }

    fn baseline_constants(&self) -> &[f64] {
        match (&self.compare, self.diff_vs_compare) {
            (Some(c), true) if c.constants.len() == self.xdf.constants.len() => &c.constants,
            _ => &self.stock_constants,
        }
    }

    fn load_compare_rom(&mut self, path: PathBuf) {
        let label = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(e) => return self.log(&format!("Compare: open failed: {}", e)),
        };
        match RomImage::from_bytes(&ext, &bytes) {
            Ok(img) => {
                // Same buffer shape as the active ROM so addresses decode identically.
                let mut buf = vec![0xFFu8; self.active_rom.len()];
                img.load_into_buffer(&mut buf, ext != "cuw");
                let cmp = crate::compare::CompareFile::decode(&label, &self.xdf, &buf, img.origin as i64);
                let (t, c) = crate::compare::diff(&self.xdf, &cmp);
                self.log(&format!(
                    "Compare: {} — {} table(s) and {} constant(s) differ",
                    label,
                    t.len(),
                    c.len()
                ));
                self.compare = Some(cmp);
                self.diff_vs_compare = true;
            }
            Err(e) => self.log(&format!("Compare: {}", e)),
        }
    }

    fn render_compare_bar(&mut self, ui: &mut egui::Ui) {
        let Some(cmp) = &self.compare else {
            return;
        };
        let name = cmp.name.clone();
        let (tdiffs, cdiffs) = crate::compare::diff(&self.xdf, cmp);
        let sel_idx = self.selected_table_idx;
        let this_differs = tdiffs.iter().any(|d| d.table_idx == sel_idx);
        let mut jump: Option<usize> = None;
        let mut copy_table = false;
        let mut close = false;

        egui::Frame::none()
            .fill(ACCENT_CYAN.gamma_multiply(0.06))
            .stroke(Stroke::new(1.0, ACCENT_CYAN.gamma_multiply(0.5)))
            .rounding(egui::Rounding::same(4.0))
            .inner_margin(egui::Margin::symmetric(8.0, 6.0))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new(format!("Comparing with {}", name)).strong().color(ACCENT_CYAN));
                    ui.label(egui::RichText::new("Deltas vs").color(MUTED));
                    ui.selectable_value(&mut self.diff_vs_compare, true, "Compare file");
                    ui.selectable_value(&mut self.diff_vs_compare, false, "Stock (as opened)");
                    ui.label(
                        egui::RichText::new(format!("{} tables, {} constants differ", tdiffs.len(), cdiffs.len()))
                            .color(if tdiffs.is_empty() && cdiffs.is_empty() { EMERALD } else { Color32::from_rgb(255, 214, 0) }),
                    );
                    if ui
                        .add_enabled(this_differs, egui::Button::new("Copy this table from compare"))
                        .on_hover_text("Replace the current table with the compare file's values (undoable)")
                        .clicked()
                    {
                        copy_table = true;
                    }
                    if ui.button("Close compare").clicked() {
                        close = true;
                    }
                });
                if !tdiffs.is_empty() || !cdiffs.is_empty() {
                    egui::CollapsingHeader::new(egui::RichText::new("Differences").small().color(MUTED))
                        .id_salt("compare_list")
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical().max_height(160.0).show(ui, |ui| {
                                for d in &tdiffs {
                                    let text = format!(
                                        "{}  \u{2014}  {}/{} cells, max \u{0394} {:.2}",
                                        d.title, d.changed_cells, d.total_cells, d.max_abs_delta
                                    );
                                    if ui.selectable_label(d.table_idx == sel_idx, text).clicked() {
                                        jump = Some(d.table_idx);
                                    }
                                }
                                for c in &cdiffs {
                                    ui.label(format!("{}: {:.2} (compare {:.2})", c.title, c.current, c.compare));
                                }
                            });
                        });
                }
            });

        if let Some(i) = jump {
            self.selected_table_idx = i;
            self.active_cell = (0, 0);
            self.selection_anchor = None;
            self.live_trace.clear();
        }
        if copy_table {
            if let Some(src) = self.compare.as_ref().and_then(|c| c.tables.get(sel_idx)).cloned() {
                self.edit_selected_table("copy table from compare", |v, _| {
                    for (row, srow) in v.iter_mut().zip(src.iter()) {
                        for (cell, s) in row.iter_mut().zip(srow.iter()) {
                            *cell = *s;
                        }
                    }
                });
            }
        }
        if close {
            self.compare = None;
            self.diff_vs_compare = false;
        }
    }
}

// ---------------------------------------------------------------------------
// Dashboard gauges
// ---------------------------------------------------------------------------

const GAUGE_SIZE: Vec2 = Vec2::new(150.0, 122.0);
const GAUGE_ALARM: Color32 = Color32::from_rgb(0xF8, 0x51, 0x49);

fn gauge_value_text(v: f64) -> String {
    if !v.is_finite() {
        "--".to_string()
    } else if v.abs() >= 100.0 {
        format!("{:.0}", v)
    } else if v.abs() >= 10.0 {
        format!("{:.1}", v)
    } else {
        format!("{:.2}", v)
    }
}

/// One dashboard gauge (dial / bar / digital). Turns red past its warning thresholds.
fn paint_gauge(ui: &mut egui::Ui, spec: &crate::gauges::GaugeSpec, label: &str, unit: &str, v: f64, live: bool) {
    use crate::gauges::GaugeStyle;
    let (rect, _) = ui.allocate_exact_size(GAUGE_SIZE, egui::Sense::hover());
    let p = ui.painter_at(rect);
    let alarm = v.is_finite() && spec.alarm(v);
    let accent = if alarm { GAUGE_ALARM } else { ACCENT_CYAN };
    let dim = if live { 1.0 } else { 0.55 };
    p.rect_filled(rect, 6.0, CARD_FILL);
    p.rect_stroke(rect, 6.0, Stroke::new(if alarm { 2.0 } else { 1.0 }, if alarm { GAUGE_ALARM } else { STROKE }));
    p.text(rect.center_bottom() - Vec2::new(0.0, 8.0), egui::Align2::CENTER_BOTTOM, label, egui::FontId::proportional(10.5), MUTED);
    let text = gauge_value_text(v);
    let frac = spec.fraction(v);

    match spec.style {
        GaugeStyle::Dial => {
            let c = egui::pos2(rect.center().x, rect.top() + 50.0);
            let r = 36.0;
            // 240° sweep from lower-left (150°) clockwise to lower-right (30°), screen coords.
            let at = |t: f32, rad: f32| {
                let a = (150.0 + 240.0 * t).to_radians();
                c + Vec2::new(a.cos(), a.sin()) * rad
            };
            let arc = |t0: f32, t1: f32, rad: f32| -> Vec<egui::Pos2> {
                let n = ((t1 - t0) * 48.0).ceil().max(2.0) as usize;
                (0..=n).map(|i| at(t0 + (t1 - t0) * i as f32 / n as f32, rad)).collect()
            };
            p.add(egui::Shape::line(arc(0.0, 1.0, r), Stroke::new(6.0, theme::WIDGET_BG)));
            if let Some(h) = spec.warn_high {
                p.add(egui::Shape::line(arc(spec.fraction(h), 1.0, r), Stroke::new(6.0, GAUGE_ALARM.gamma_multiply(0.45))));
            }
            if let Some(l) = spec.warn_low {
                p.add(egui::Shape::line(arc(0.0, spec.fraction(l), r), Stroke::new(6.0, GAUGE_ALARM.gamma_multiply(0.45))));
            }
            if v.is_finite() && frac > 0.0 {
                p.add(egui::Shape::line(arc(0.0, frac, r), Stroke::new(6.0, accent.gamma_multiply(dim))));
            }
            for k in 0..=4 {
                let t = k as f32 / 4.0;
                p.line_segment([at(t, r + 5.0), at(t, r + 9.0)], Stroke::new(1.0, MUTED));
            }
            // Range labels sit outside the arc ends so they never touch the readout.
            p.text(at(0.0, r + 6.0) + Vec2::new(-2.0, 0.0), egui::Align2::RIGHT_CENTER, gauge_value_text(spec.min), egui::FontId::monospace(8.5), MUTED);
            p.text(at(1.0, r + 6.0) + Vec2::new(2.0, 0.0), egui::Align2::LEFT_CENTER, gauge_value_text(spec.max), egui::FontId::monospace(8.5), MUTED);
            if v.is_finite() {
                p.line_segment([c, at(frac, r - 8.0)], Stroke::new(2.0, TEXT.gamma_multiply(dim)));
            }
            p.circle_filled(c, 3.5, TEXT);
            p.text(c + Vec2::new(0.0, 24.0), egui::Align2::CENTER_CENTER, &text, egui::FontId::monospace(16.0), accent.gamma_multiply(dim));
            p.text(c + Vec2::new(0.0, 38.0), egui::Align2::CENTER_CENTER, unit, egui::FontId::proportional(9.0), MUTED);
        }
        GaugeStyle::Bar => {
            p.text(rect.center() - Vec2::new(0.0, 22.0), egui::Align2::CENTER_CENTER, &text, egui::FontId::monospace(20.0), accent.gamma_multiply(dim));
            p.text(rect.center() - Vec2::new(0.0, 6.0), egui::Align2::CENTER_CENTER, unit, egui::FontId::proportional(9.0), MUTED);
            let bar = egui::Rect::from_min_size(rect.left_center() + Vec2::new(12.0, 8.0), Vec2::new(rect.width() - 24.0, 14.0));
            p.rect_filled(bar, 3.0, theme::WIDGET_BG);
            let x_of = |t: f32| bar.left() + bar.width() * t;
            if let Some(h) = spec.warn_high {
                p.rect_filled(egui::Rect::from_x_y_ranges(x_of(spec.fraction(h))..=bar.right(), bar.y_range()), 3.0, GAUGE_ALARM.gamma_multiply(0.35));
            }
            if let Some(l) = spec.warn_low {
                p.rect_filled(egui::Rect::from_x_y_ranges(bar.left()..=x_of(spec.fraction(l)), bar.y_range()), 3.0, GAUGE_ALARM.gamma_multiply(0.35));
            }
            if v.is_finite() {
                // Bipolar ranges (fuel trims) fill from zero, not from the minimum.
                let zero = if spec.min < 0.0 && spec.max > 0.0 { spec.fraction(0.0) } else { 0.0 };
                let (a, b) = if frac >= zero { (zero, frac) } else { (frac, zero) };
                p.rect_filled(egui::Rect::from_x_y_ranges(x_of(a)..=x_of(b), bar.y_range()), 3.0, accent.gamma_multiply(dim));
            }
            p.text(bar.left_bottom() + Vec2::new(0.0, 2.0), egui::Align2::LEFT_TOP, gauge_value_text(spec.min), egui::FontId::monospace(8.5), MUTED);
            p.text(bar.right_bottom() + Vec2::new(0.0, 2.0), egui::Align2::RIGHT_TOP, gauge_value_text(spec.max), egui::FontId::monospace(8.5), MUTED);
        }
        GaugeStyle::Digital => {
            p.text(rect.center() - Vec2::new(0.0, 8.0), egui::Align2::CENTER_CENTER, &text, egui::FontId::monospace(28.0), accent.gamma_multiply(dim));
            p.text(rect.center() + Vec2::new(0.0, 16.0), egui::Align2::CENTER_CENTER, unit, egui::FontId::proportional(10.0), MUTED);
        }
    }
}

impl WidebandApp {
    fn render_dashboard_editor(&mut self, ui: &mut egui::Ui, channels: &[crate::telemetry::TelemetryChannelDef]) {
        use crate::gauges::{Dashboard, GaugeSpec, GaugeStyle};
        let mut remove: Option<usize> = None;
        let mut swap: Option<(usize, usize)> = None;
        let n = self.dashboard.gauges.len();
        egui::Frame::none()
            .fill(CARD_FILL)
            .stroke(Stroke::new(1.0, STROKE))
            .inner_margin(egui::Margin::same(6.0))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                    egui::Grid::new("dash_edit").striped(true).spacing([8.0, 4.0]).show(ui, |ui| {
                        for h in ["Channel", "Style", "Min", "Max", "Warn low", "Warn high", ""] {
                            ui.label(egui::RichText::new(h).small().color(MUTED));
                        }
                        ui.end_row();
                        for (i, g) in self.dashboard.gauges.iter_mut().enumerate() {
                            let shown = channels.iter().find(|c| c.id == g.channel).map(|c| c.label.clone()).unwrap_or(g.channel.clone());
                            egui::ComboBox::from_id_salt(("g_ch", i)).selected_text(shown).width(150.0).show_ui(ui, |ui| {
                                for c in channels {
                                    ui.selectable_value(&mut g.channel, c.id.clone(), &c.label);
                                }
                            });
                            egui::ComboBox::from_id_salt(("g_st", i)).selected_text(g.style.label()).width(70.0).show_ui(ui, |ui| {
                                for st in GaugeStyle::ALL {
                                    ui.selectable_value(&mut g.style, st, st.label());
                                }
                            });
                            ui.add(egui::DragValue::new(&mut g.min).speed(1.0));
                            ui.add(egui::DragValue::new(&mut g.max).speed(1.0));
                            for (slot, default) in [(&mut g.warn_low, g.min), (&mut g.warn_high, g.max)] {
                                ui.horizontal(|ui| {
                                    let mut on = slot.is_some();
                                    if ui.checkbox(&mut on, "").changed() {
                                        *slot = on.then_some(default);
                                    }
                                    if let Some(v) = slot {
                                        ui.add(egui::DragValue::new(v).speed(0.5));
                                    }
                                });
                            }
                            ui.horizontal(|ui| {
                                if ui.add_enabled(i > 0, egui::Button::new("\u{2191}").small()).clicked() {
                                    swap = Some((i, i - 1));
                                }
                                if ui.add_enabled(i + 1 < n, egui::Button::new("\u{2193}").small()).clicked() {
                                    swap = Some((i, i + 1));
                                }
                                if ui.small_button("Remove").clicked() {
                                    remove = Some(i);
                                }
                            });
                            ui.end_row();
                        }
                    });
                });
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("g_add").selected_text("Add gauge\u{2026}").show_ui(ui, |ui| {
                        for c in channels {
                            if ui.selectable_label(false, &c.label).clicked() {
                                self.dashboard.gauges.push(GaugeSpec::default_for(&c.id));
                            }
                        }
                    });
                    if ui.button("Save layout").clicked() {
                        let path = Dashboard::save_path();
                        match self.dashboard.save(&path) {
                            Ok(()) => self.log(&format!("Dashboard saved to {}", path.display())),
                            Err(e) => self.log(&format!("Dashboard save failed: {}", e)),
                        }
                    }
                    if ui.button("Reset to defaults").clicked() {
                        let ids: Vec<String> = channels.iter().map(|c| c.id.clone()).collect();
                        self.dashboard = Dashboard::default_layout(&ids);
                    }
                });
            });
        if let Some((a, b)) = swap {
            self.dashboard.gauges.swap(a, b);
        }
        if let Some(i) = remove {
            self.dashboard.gauges.remove(i);
        }
    }
}

// ---------------------------------------------------------------------------
// .wbtune tune files
// ---------------------------------------------------------------------------

impl WidebandApp {
    fn save_tune(&mut self, path: PathBuf) {
        // Same as Save ROM: make sure every edit is in the image before it's captured.
        self.xdf.write_all_tables_to_rom(&mut self.active_rom, self.rom_origin);
        self.xdf.write_all_constants_to_rom(&mut self.active_rom, self.rom_origin);
        for flag in &self.xdf.flags {
            self.xdf.write_flag_to_rom(&mut self.active_rom, self.rom_origin, flag);
        }
        let prof_id = self.profiles[self.selected_profile_idx].id.clone();
        let input = crate::tune_file::SaveInput {
            profile_id: &prof_id,
            vin: &self.ecu_id.vin,
            cal_id: &self.ecu_id.cal_id,
            notes: &self.tune_notes,
            rom: &self.active_rom,
            rom_origin: self.rom_origin,
            xdf_path: self.xdf_path.as_deref(),
            xdf_title: &self.xdf.title,
        };
        match crate::tune_file::save(&path, &input) {
            Ok(()) => {
                let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                if self.xdf_path.is_none() {
                    self.log(&format!(
                        "Saved tune {} (definition came from a profile pack, so it isn't referenced; reopen with the same pack)",
                        name
                    ));
                } else {
                    self.log(&format!("Saved tune {}", name));
                }
            }
            Err(e) => self.log(&format!("Save tune failed: {}", e)),
        }
    }

    fn open_tune(&mut self, path: PathBuf) {
        let opened = match crate::tune_file::open(&path) {
            Ok(o) => o,
            Err(e) => return self.log(&format!("Open tune failed: {}", e)),
        };
        let t = &opened.tune;
        if self.profile_index_for_id(&t.profile_id).is_some() {
            self.select_profile_id(&t.profile_id);
        } else {
            self.log(&format!("Tune was saved for profile '{}', which isn't installed", t.profile_id));
        }
        match &opened.xdf {
            crate::tune_file::XdfStatus::Match(p) => self.load_xdf_from_path(p.clone()),
            crate::tune_file::XdfStatus::Changed(p) => {
                self.log(&format!(
                    "Warning: {} changed since this tune was saved; table addresses or scaling may differ",
                    p.display()
                ));
                self.load_xdf_from_path(p.clone());
            }
            crate::tune_file::XdfStatus::Missing => self.log(&format!(
                "Definition \"{}\" not found; open the matching XDF to edit this tune",
                t.xdf_title
            )),
        }
        self.active_rom = opened.rom;
        self.rom_origin = t.rom_origin;
        self.rom_path = Some(path.clone());
        self.rom_from_ecu = false;
        self.tune_notes = t.notes.clone();
        self.compare = None;
        self.diff_vs_compare = false;
        self.xdf.bind_from_rom(&self.active_rom, self.rom_origin);
        self.refresh_stock_baseline();
        self.table_undo.clear();
        self.refresh_checksum_badge();
        self.log(&format!(
            "Opened tune {} (saved {}{}{})",
            path.file_name().unwrap_or_default().to_string_lossy(),
            t.saved_utc,
            if t.vin.is_empty() { String::new() } else { format!(", VIN {}", t.vin) },
            if t.cal_id.is_empty() { String::new() } else { format!(", cal {}", t.cal_id) },
        ));
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CalTool {
    Histogram,
    FuelCorrection,
    Notes,
}
