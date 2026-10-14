use crate::audit_log::FlashAuditLog;
use crate::cal_editor::{
    cell_changed, format_cell_display, param_matches_query, table_has_edits, FavoriteTables,
};
use crate::definitions::{RomImage, XdfDocument};
use crate::diagnostics::{
    self, DiagnosticsSnapshot, DtcStatus, status_label,
};
use crate::flash_job::FlashJob;
use crate::flash_layout::FlashLayoutMap;
use crate::j2534::{
    connect_for_profile, create_device, transport_hint_for_device, J2534DeviceInfo, J2534RegistryScanner,
    PassThruDevice, READ_VBATT,
};
#[cfg(windows)]
use crate::j2534_host::HostPassThruDevice;
use crate::protocols::{EcuIdentification, EcuIdentifier, EcuProfile};
use crate::tcm::{
    build_parameter_view, flash_blockers as tcm_flash_blockers, group_label, TcmIdentifier,
    TcmParameterGroup, TcmParameterView, TcmProfile, TcmProfileCatalog,
};
use crate::security_config::{
    key_registration_label, security_mode_label, PlatformSecurity, KeyRegistration, SecurityMode,
    CHECKSUM_NOT_CONFIGURED, CHECKSUM_VALID,
};
use std::fs;
use std::path::PathBuf;
use crate::surface_mesh;
use crate::telemetry::{decode_channel_value, TelemetryBuffer, TelemetryChannelFile};
use crate::theme::{
    self, paint_live_cell_glow, paint_studio_badge, studio_tab_button, ACCENT_CYAN, CARD_FILL,
    EMERALD, MUTED, STROKE, TEXT, heatmap_color,
};
use eframe::egui::{self, Color32, Stroke, Vec2};
use egui_extras::{Column, TableBuilder};
use egui_plot::{Line, Plot, PlotPoints};
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceTab {
    Calibration,
    Telemetry,
    Transmission,
    FlashDiagnostics,
    Switchboard,
}

pub struct WidebandApp {
    // Hardware & Protocols
    pub devices: Vec<J2534DeviceInfo>,
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
    pub stock_flags: Vec<bool>,
    pub favorites: FavoriteTables,
    pub favorites_path: PathBuf,
    pub favorites_only: bool,
    pub active_cell: (usize, usize),
    pub math_amount: f64,
    pub checksum_status: String,
    pub active_rom: Vec<u8>,
    pub rom_origin: i64,
    pub rom_path: Option<PathBuf>,
    pub xdf_path: Option<PathBuf>,

    // Telemetry
    pub telemetry: Arc<Mutex<TelemetryBuffer>>,
    pub trace_rpm: f32,
    pub trace_map: f32,
    pub live_poller_active: bool,
    pub last_poll_time: Instant,

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

    // TCM (paired with ECM profile)
    pub tcm_profiles: Vec<TcmProfile>,
    pub selected_tcm_profile_idx: usize,
    pub tcm_id: EcuIdentification,
    pub tcm_identify_error: String,
    pub tcm_param_view: TcmParameterView,
    pub tcm_flash_confirm_open: bool,

    // UI Workspace
    pub active_tab: WorkspaceTab,
}

impl WidebandApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply_motorsport_theme(&cc.egui_ctx);
        let devices = J2534RegistryScanner::scan();
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
                FlashLayoutMap::load("tests/flash-layout-test.json").expect("flash layout fallback")
            });
        let flash_audit = FlashAuditLog::new(FlashAuditLog::default_path());

        let mut app = Self {
            devices,
            selected_device_idx: 0,
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
            stock_flags,
            favorites,
            favorites_path,
            favorites_only: false,
            active_cell: (0, 0),
            math_amount: 5.0,
            checksum_status: CHECKSUM_VALID.to_string(),
            active_rom: rom,
            rom_origin: 0,
            rom_path: None,
            xdf_path: None,
            telemetry: Arc::new(Mutex::new(
                TelemetryBuffer::from_definition(&TelemetryChannelFile::load_default(), None),
            )),
            trace_rpm: 2400.0,
            trace_map: 70.0,
            live_poller_active: true,
            last_poll_time: Instant::now(),
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
            tcm_profiles,
            selected_tcm_profile_idx: 0,
            tcm_id: EcuIdentification::default(),
            tcm_identify_error: String::new(),
            tcm_param_view,
            tcm_flash_confirm_open: false,
            active_tab: WorkspaceTab::Calibration,
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

        // Auto-connect to Virtual ECU Loopback on startup for instant live preview
        app.connect_cable();
        app
    }

    fn commit_table_edits(&mut self) {
        let idx = self.selected_table_idx;
        self.xdf.write_table_to_rom(&mut self.active_rom, self.rom_origin, idx);
        self.refresh_checksum_badge();
    }

    fn refresh_stock_baseline(&mut self) {
        self.stock_tables = self.xdf.tables.iter().map(|t| t.values.clone()).collect();
        self.stock_constants = self.xdf.constants.iter().map(|c| c.value).collect();
        self.stock_flags = self.xdf.flags.iter().map(|f| f.is_set).collect();
    }

    fn refresh_checksum_badge(&mut self) {
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
                    let label = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                    self.xdf_path = Some(path);
                    self.xdf.bind_from_rom(&self.active_rom, self.rom_origin);
                    self.refresh_stock_baseline();
                    self.refresh_checksum_badge();
                    self.log(&format!("Loaded XDF {} ({} tables)", label, self.xdf.tables.len()));
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

    pub fn connect_cable(&mut self) {
        self.connection_last_error.clear();
        self.header_vbatt = None;
        if self.selected_device_idx >= self.devices.len() {
            self.connection_last_error = "No cable device selected.".to_string();
            return;
        }
        let dev_info = self.devices[self.selected_device_idx].clone();
        let is_virtual = dev_info.is_virtual;
        let dev_name = dev_info.list_label();
        let transport = transport_hint_for_device(&dev_info);

        if is_virtual {
            self.log(
                "Virtual ECU selected: responses are synthetic bench data, not your Tacoma on a real Pass-Thru cable.",
            );
        }

        if !is_virtual && !dev_info.exists {
            self.connection_last_error =
                format!("PassThru DLL not found: {}", dev_info.library_path);
            self.log(&format!(
                "{}. Install the Mini-VCI or VXDIAG (Toyota) J2534 driver, then rescan devices.",
                self.connection_last_error
            ));
            return;
        }

        #[cfg(windows)]
        if !is_virtual && !HostPassThruDevice::host_available() {
            self.log(
                "J2534Host.exe is missing beside wideband.exe. Build it from J2534Host.cs (x86) — required for most 32-bit cable DLLs.",
            );
        }

        self.log(&format!("Opening {} via {}", dev_name, transport));

        let mut dev = create_device(&dev_info);
        match dev.open() {
            Err(e) => {
                self.connection_last_error = e.clone();
                self.log(&format!(
                    "Failed to open {}: {} (check USB, driver, and VX Manager / MVCI license if applicable)",
                    dev_name,
                    e
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
        self.connection_last_error.clear();
    }

    pub fn disconnect_cable(&mut self) {
        if let Some(mut dev) = self.active_device.take() {
            let _ = dev.close();
            self.is_connected = false;
            self.header_vbatt = None;
            PlatformSecurity::install(&PlatformSecurity::default_config_path());
            self.log("Cable disconnected.");
        }
    }

    fn cable_is_virtual(&self) -> bool {
        if self.is_connected {
            self.active_device_virtual
        } else {
            self.devices
                .get(self.selected_device_idx)
                .map(|d| d.is_virtual)
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
            {
                let mut buf = self.telemetry.lock().unwrap();
                self.trace_rpm = buf.value("rpm");
                self.trace_map = buf.value("map");
                buf.push_frame();
            }

            // Update tracked active cell in active table
            if !self.xdf.tables.is_empty() {
                let table = &self.xdf.tables[self.selected_table_idx];
                let mut best_col = 0;
                let mut min_diff_x = f32::MAX;
                for (c, &lbl) in table.x_axis.labels.iter().enumerate() {
                    let diff = (lbl as f32 - self.trace_rpm).abs();
                    if diff < min_diff_x {
                        min_diff_x = diff;
                        best_col = c;
                    }
                }

                let mut best_row = 0;
                let mut min_diff_y = f32::MAX;
                for (r, &lbl) in table.y_axis.labels.iter().enumerate() {
                    let diff = (lbl as f32 - self.trace_map).abs();
                    if diff < min_diff_y {
                        min_diff_y = diff;
                        best_row = r;
                    }
                }
                self.active_cell = (best_row, best_col);
            }
        }
    }

    pub fn run_table_math(&mut self, op: char) {
        if self.xdf.tables.is_empty() {
            return;
        }
        let table = &mut self.xdf.tables[self.selected_table_idx];
        let (r, c) = self.active_cell;
        if r < table.values.len() && c < table.values[r].len() {
            let cur = table.values[r][c];
            let updated = match op {
                '+' => cur + self.math_amount,
                '-' => cur - self.math_amount,
                '*' => cur * (1.0 + (self.math_amount / 100.0)),
                '%' => cur * (self.math_amount / 100.0),
                '=' => self.math_amount,
                _ => cur,
            };
            table.values[r][c] = (updated * 100.0).round() / 100.0;
            self.commit_table_edits();
        }
    }

    pub fn interpolate_active_row(&mut self) {
        if self.xdf.tables.is_empty() {
            return;
        }
        let table = &mut self.xdf.tables[self.selected_table_idx];
        let r = self.active_cell.0;
        if r < table.values.len() && table.values[r].len() >= 2 {
            let row = &mut table.values[r];
            let first = row[0];
            let last = *row.last().unwrap();
            let n = row.len();
            for i in 0..n {
                let frac = i as f64 / (n - 1) as f64;
                row[i] = ((first + (last - first) * frac) * 100.0).round() / 100.0;
            }
            self.commit_table_edits();
        }
    }

    pub fn smooth_table(&mut self) {
        if self.xdf.tables.is_empty() {
            return;
        }
        let table = &mut self.xdf.tables[self.selected_table_idx];
        let rows = table.values.len();
        let cols = table.values[0].len();
        let mut smoothed = table.values.clone();

        for r in 1..rows - 1 {
            for c in 1..cols - 1 {
                let avg = (table.values[r - 1][c]
                    + table.values[r + 1][c]
                    + table.values[r][c - 1]
                    + table.values[r][c + 1]
                    + table.values[r][c] * 4.0)
                    / 8.0;
                smoothed[r][c] = (avg * 100.0).round() / 100.0;
            }
        }
        table.values = smoothed;
        self.commit_table_edits();
    }

    fn render_studio_header(&mut self, ctx: &egui::Context, tier: theme::StudioHeaderTier) {
        use theme::StudioHeaderTier;
        let compact_badge = tier != StudioHeaderTier::Wide;
        let tabs_compact = tier != StudioHeaderTier::Wide;
        let stacked = tier != StudioHeaderTier::Wide;
        let header_h = if stacked { 84.0 } else { 48.0 };
        let profile_combo_w = match tier {
            StudioHeaderTier::Compact => 130.0,
            StudioHeaderTier::Medium => 170.0,
            StudioHeaderTier::Wide => 200.0,
        };
        let device_combo_w = match tier {
            StudioHeaderTier::Compact => 110.0,
            StudioHeaderTier::Medium => 140.0,
            StudioHeaderTier::Wide => 160.0,
        };
        egui::TopBottomPanel::top("header_bar")
            .exact_height(header_h)
            .frame(theme::header_frame())
            .show(ctx, |ui| {
                if stacked {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            paint_studio_badge(ui, compact_badge);
                            ui.add_space(8.0);
                            self.paint_profile_selector(ui, profile_combo_w);
                            self.paint_header_status_chips(ui);
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                self.paint_connect_controls(ui, device_combo_w);
                            });
                        });
                        self.paint_workspace_tabs(ui, tabs_compact);
                    });
                } else {
                    ui.horizontal(|ui| {
                        paint_studio_badge(ui, false);
                        ui.add_space(12.0);
                        self.paint_profile_selector(ui, profile_combo_w);
                        ui.add_space(8.0);
                        self.paint_header_status_chips(ui);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            self.paint_connect_controls(ui, device_combo_w);
                            ui.add_space(10.0);
                            self.paint_workspace_tabs(ui, false);
                        });
                    });
                }
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

    fn paint_profile_selector(&mut self, ui: &mut egui::Ui, width: f32) {
        ui.label(egui::RichText::new("Vehicle").size(11.0).color(MUTED));
        let prev_ecm = self.selected_profile_idx;
        egui::ComboBox::from_id_salt("profile_select")
            .width(width)
            .selected_text(&self.profiles[self.selected_profile_idx].name)
            .show_ui(ui, |ui| {
                for (idx, prof) in self.profiles.iter().enumerate() {
                    ui.selectable_value(&mut self.selected_profile_idx, idx, &prof.name);
                }
            });
        if prev_ecm != self.selected_profile_idx {
            self.sync_tcm_profile_to_ecm();
        }
    }

    fn paint_connect_controls(&mut self, ui: &mut egui::Ui, device_combo_w: f32) {
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
    }

    fn paint_workspace_tabs(&mut self, ui: &mut egui::Ui, compact: bool) {
        let tabs = [
            (WorkspaceTab::Switchboard, "Switchboard"),
            (WorkspaceTab::FlashDiagnostics, "Flash & Diag"),
            (WorkspaceTab::Transmission, "Transmission"),
            (WorkspaceTab::Telemetry, "Telemetry"),
            (WorkspaceTab::Calibration, "Calibration"),
        ];
        ui.horizontal_wrapped(|ui| {
            for (tab, title) in tabs {
                if studio_tab_button(ui, title, self.active_tab == tab, compact).clicked() {
                    self.active_tab = tab;
                }
                ui.add_space(4.0);
            }
        });
    }

    fn paint_live_telemetry_card(&self, ui: &mut egui::Ui) {
        egui::Frame::none()
            .fill(CARD_FILL)
            .stroke(Stroke::new(1.0, STROKE))
            .rounding(egui::Rounding::same(4.0))
            .inner_margin(egui::Margin::symmetric(10.0, 8.0))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.columns(2, |cols| {
                    cols[0].label(
                        egui::RichText::new("Live cell tracking")
                            .size(11.0)
                            .strong()
                            .color(MUTED),
                    );
                    cols[1].with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(egui::RichText::new("● LIVE").size(11.0).strong().color(EMERALD));
                    });
                });
                ui.add_space(6.0);
                ui.label(egui::RichText::new("Engine RPM").size(10.0).color(MUTED));
                ui.label(
                    egui::RichText::new(format!("{:.0}", self.trace_rpm))
                        .size(14.0)
                        .monospace()
                        .strong()
                        .color(ACCENT_CYAN),
                );
                ui.add_space(2.0);
                ui.label(egui::RichText::new("MAP").size(10.0).color(MUTED));
                ui.label(
                    egui::RichText::new(format!("{:.1} kPa", self.trace_map))
                        .size(14.0)
                        .monospace()
                        .strong()
                        .color(ACCENT_CYAN),
                );
                ui.add_space(2.0);
                ui.label(egui::RichText::new("Active tracked cell").size(10.0).color(MUTED));
                ui.label(
                    egui::RichText::new(format!("Row {}, Col {}", self.active_cell.0, self.active_cell.1))
                        .size(12.0)
                        .monospace()
                        .strong()
                        .color(ACCENT_CYAN),
                );
            });
    }

    fn paint_header_status_chips(&self, ui: &mut egui::Ui) {
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
                        ui.label(
                            egui::RichText::new(format!("— {}", self.connection_last_error))
                                .size(10.0)
                                .color(Color32::from_rgb(0xF8, 0x51, 0x49)),
                        );
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
                        egui::RichText::new("Virtual ECU — not a real OBD cable")
                            .size(10.0)
                            .strong()
                            .color(Color32::from_rgb(255, 193, 7)),
                    );
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
        ctx.request_repaint();

        theme::apply_motorsport_theme(ctx);

        let tier = theme::studio_header_tier(ctx);
        self.render_studio_header(ctx, tier);

        // -------------------------------------------------------------------
        // Workspace View Rendering
        // -------------------------------------------------------------------
        match self.active_tab {
            WorkspaceTab::Calibration => self.render_calibration_tab(ctx),
            WorkspaceTab::Telemetry => self.render_telemetry_tab(ctx),
            WorkspaceTab::Transmission => self.render_transmission_tab(ctx),
            WorkspaceTab::FlashDiagnostics => self.render_flash_tab(ctx),
            WorkspaceTab::Switchboard => self.render_switchboard_tab(ctx),
        }
    }
}

// ---------------------------------------------------------------------------
// Tab 1: Calibration (Parameter Tree + 2D/3D Heatmap Grid + 3D Surface Mesh)
// ---------------------------------------------------------------------------

impl WidebandApp {
    fn render_calibration_tab(&mut self, ctx: &egui::Context) {
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
                                        .stock_tables
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
                                                self.active_cell = (0, 0);
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
                                    let basis = self.stock_constants.get(i).copied().unwrap_or(c.value);
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
                        egui::RichText::new("Drag: orbit · Scroll: zoom")
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
                    ui.allocate_painter(mesh_size, egui::Sense::drag());
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
                    surface_mesh::paint_surface_mesh(
                        &painter,
                        response.rect,
                        table,
                        self.mesh_yaw,
                        self.mesh_pitch,
                        self.mesh_zoom,
                        self.active_cell,
                    );
                } else {
                    painter.rect_filled(response.rect, 4.0, theme::BG_FILL);
                }

                ui.add_space(6.0);
                self.paint_live_telemetry_card(ui);
            });

        // Center Heatmap Table Grid
        egui::CentralPanel::default()
            .frame(theme::panel_frame())
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Open ROM (.bin / .cuw)").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("ROM image", &["bin", "cuw"])
                            .pick_file()
                        {
                            self.load_rom_from_path(path);
                        }
                    }
                    if ui.button("Open XDF").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("TunerPro XDF", &["xdf"])
                            .pick_file()
                        {
                            self.load_xdf_from_path(path);
                        }
                    }
                    if ui.button("Save ROM").clicked() {
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
                });
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

                ui.horizontal_wrapped(|ui| {
                    ui.heading(egui::RichText::new(&table_title).size(16.0).color(TEXT));
                    ui.label(egui::RichText::new(format!("({})", z_units)).color(ACCENT_CYAN));
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
                    ui.add(egui::DragValue::new(&mut self.math_amount).speed(0.5).range(0.1..=100.0));

                    if ui.button("  +  ").clicked() {
                        self.run_table_math('+');
                    }
                    if ui.button("  -  ").clicked() {
                        self.run_table_math('-');
                    }
                    if ui.button("  %  ").clicked() {
                        self.run_table_math('%');
                    }
                    if ui.button("  =  ").clicked() {
                        self.run_table_math('=');
                    }
                    if ui.button("Interpolate Row").clicked() {
                        self.interpolate_active_row();
                    }
                    if ui.button("Smooth 3D").clicked() {
                        self.smooth_table();
                    }
                });
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
        let stock = &self.stock_tables[self.selected_table_idx];
        let x_labels = table.x_axis.labels.clone();
        let y_labels = table.y_axis.labels.clone();
        let rows = table.values.len();
        let cols = table.values[0].len();

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
                            let is_tracked = self.active_cell == (r, c);

                            let bg_color = heatmap_color(val, min_val, max_val);
                            let text_color = Color32::from_rgb(0xFF, 0xFF, 0xFF);
                            let cell_text = format_cell_display(val, stock_val);

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

                                if is_tracked {
                                    paint_live_cell_glow(painter, rect);
                                }

                                let response = ui.selectable_label(
                                    is_tracked,
                                    egui::RichText::new(cell_text)
                                        .monospace()
                                        .strong()
                                        .color(text_color),
                                );
                                if response.clicked() {
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
    fn render_telemetry_tab(&mut self, ctx: &egui::Context) {
        let (gauge_channels, recording, def_title, stats_line, rate_rows) = {
            let buf = self.telemetry.lock().unwrap();
            let gauges: Vec<(String, String, String, [u8; 3])> = buf
                .channels
                .iter()
                .enumerate()
                .filter(|(_, ch)| ch.gauge)
                .map(|(i, ch)| {
                    (
                        ch.label.clone(),
                        Self::format_channel_value(ch.id.as_str(), buf.latest[i]),
                        ch.unit.clone(),
                        ch.plot_color(),
                    )
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

        egui::TopBottomPanel::top("gauge_strip").min_height(72.0).show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Export CSV").clicked() {
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
                    if ui.button("Clear").clicked() {
                        self.telemetry.lock().unwrap().clear();
                        self.log("Telemetry history cleared.");
                    }
                });
                for (label, text, unit, rgb) in &gauge_channels {
                    self.render_gauge_card(
                        ui,
                        label,
                        text.clone(),
                        unit,
                        Color32::from_rgb(rgb[0], rgb[1], rgb[2]),
                    );
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.heading(
                    egui::RichText::new("Definition-Driven Live Strip Chart")
                        .size(15.0)
                        .color(Color32::WHITE),
                );
                ui.label(
                    egui::RichText::new(format!("({})", def_title))
                        .small()
                        .color(Color32::GRAY),
                );
            });
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
            ui.add_space(4.0);

            let plot_h = ui.available_size().y.max(160.0);
            let buffer = self.telemetry.lock().unwrap();
            let mut plot_lines: Vec<Line> = Vec::new();
            for (ch_idx, ch) in buffer.channels.iter().enumerate() {
                if ch.plot.is_none() {
                    continue;
                }
                let scale = ch.plot_scale();
                let mut points = Vec::new();
                for frame in &buffer.frames {
                    if ch_idx < frame.values.len() {
                        points.push([frame.time_sec, frame.values[ch_idx] as f64 * scale]);
                    }
                }
                let rgb = ch.plot_color();
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

            Plot::new("telemetry_oscilloscope")
                .height(plot_h)
                .width(ui.available_width())
                .view_aspect(ui.available_width() / plot_h.max(1.0))
                .show_axes([true, true])
                .legend(egui_plot::Legend::default())
                .show(ui, |plot_ui| {
                    for line in plot_lines {
                        plot_ui.line(line);
                    }
                });
        });
    }

    fn format_channel_value(channel_id: &str, value: f32) -> String {
        match channel_id {
            "rpm" | "ect" | "load" => format!("{:.0}", value),
            "map" | "timing" => format!("{:.1}", value),
            "lambda" => format!("{:.3}", value),
            "vbatt" => format!("{:.2}", value),
            _ => format!("{:.1}", value),
        }
    }

    fn render_gauge_card(&self, ui: &mut egui::Ui, title: &str, value: String, unit: &str, accent: Color32) {
        ui.group(|ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(title).small().color(Color32::from_rgb(140, 155, 175)));
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(value).size(18.0).strong().color(accent));
                    ui.label(egui::RichText::new(unit).small().color(Color32::GRAY));
                });
            });
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
    }
}

// ---------------------------------------------------------------------------
// Tab 3: Diagnostics & ECU Flashing
// ---------------------------------------------------------------------------

impl WidebandApp {
    fn render_flash_tab(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
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

    fn refresh_diagnostics(&mut self) {
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
