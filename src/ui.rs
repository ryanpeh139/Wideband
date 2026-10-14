use crate::audit_log::FlashAuditLog;
use crate::definitions::{CuwFile, XdfDocument};
use crate::flash_job::FlashJob;
use crate::flash_layout::FlashLayoutMap;
use crate::j2534::{connect_for_profile, create_device, J2534DeviceInfo, J2534RegistryScanner, PassThruDevice};
use crate::protocols::{EcuIdentification, EcuIdentifier, EcuProfile};
use crate::security_config::{PlatformSecurity, CHECKSUM_UPDATED, CHECKSUM_VALID};
use std::fs;
use std::path::PathBuf;
use crate::surface_mesh;
use crate::telemetry::{decode_channel_value, TelemetryBuffer, TelemetryChannelFile};
use crate::theme::{
    self, paint_live_cell_glow, paint_studio_badge, studio_tab_button, ACCENT_CYAN, CARD_FILL,
    EMERALD, MUTED, STROKE, TEXT, heatmap_color,
};
use eframe::egui::{self, Color32, Pos2, Stroke, Vec2};
use egui_extras::{Column, TableBuilder};
use egui_plot::{Line, Plot, PlotPoints};
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceTab {
    Calibration,
    Telemetry,
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
    pub is_connected: bool,

    // Calibration & XDF
    pub xdf: XdfDocument,
    pub selected_table_idx: usize,
    pub search_query: String,
    pub stock_tables: Vec<Vec<Vec<f64>>>,
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

    // Flash & Diagnostics
    pub ecu_id: EcuIdentification,
    pub comm_log: Vec<String>,
    pub transfer_progress: f32,
    pub is_flashing: bool,
    pub flash_layout: FlashLayoutMap,
    pub flash_audit: FlashAuditLog,
    pub flash_status_detail: String,

    // UI Workspace
    pub active_tab: WorkspaceTab,
}

impl WidebandApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply_motorsport_theme(&cc.egui_ctx);
        let devices = J2534RegistryScanner::scan();
        let profiles = EcuProfile::all();
        let xdf = XdfDocument::sample();

        let stock_tables = xdf.tables.iter().map(|t| t.values.clone()).collect();
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
            is_connected: false,
            xdf,
            selected_table_idx: 0,
            search_query: String::new(),
            stock_tables,
            active_cell: (0, 0),
            math_amount: 5.0,
            checksum_status: CHECKSUM_VALID.to_string(),
            active_rom: rom,
            rom_origin: 0,
            rom_path: None,
            xdf_path: None,
            telemetry: Arc::new(Mutex::new(
                TelemetryBuffer::from_definition(&TelemetryChannelFile::load_default(), 5000),
            )),
            trace_rpm: 2400.0,
            trace_map: 70.0,
            live_poller_active: true,
            last_poll_time: Instant::now(),
            mesh_yaw: 0.785,  // 45 degrees
            mesh_pitch: 0.523, // 30 degrees
            ecu_id: EcuIdentification::default(),
            comm_log: vec!["Wideband Motorsport Studio 2.4 (Rust Native) Initialized.".to_string()],
            transfer_progress: 0.0,
            is_flashing: false,
            flash_layout,
            flash_audit,
            flash_status_detail: String::new(),
            active_tab: WorkspaceTab::Calibration,
        };

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
        self.checksum_status = CHECKSUM_UPDATED.to_string();
    }

    fn refresh_stock_tables(&mut self) {
        self.stock_tables = self.xdf.tables.iter().map(|t| t.values.clone()).collect();
    }

    fn load_rom_from_path(&mut self, path: PathBuf) {
        let label = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let load_result: Result<String, String> = if ext == "cuw" {
            let bytes = match fs::read(&path) {
                Ok(b) => b,
                Err(e) => return self.log(&format!("Open ROM failed: {}", e)),
            };
            let cuw = match CuwFile::parse(&bytes) {
                Ok(c) => c,
                Err(e) => return self.log(&format!("Open ROM failed: {}", e)),
            };
            self.rom_origin = cuw.origin as i64;
            if cuw.image.len() > self.active_rom.len() {
                self.active_rom.resize(cuw.image.len(), 0xFF);
            }
            let start = cuw.origin as usize;
            if start + cuw.image.len() <= self.active_rom.len() {
                self.active_rom[start..start + cuw.image.len()].copy_from_slice(&cuw.image);
            }
            Ok(format!(
                "Loaded CUW {} ({} bytes @ 0x{:X})",
                label,
                cuw.image.len(),
                cuw.origin
            ))
        } else {
            let data = match fs::read(&path) {
                Ok(d) => d,
                Err(e) => return self.log(&format!("Open ROM failed: {}", e)),
            };
            self.active_rom = data;
            self.rom_origin = 0;
            Ok(format!("Loaded ROM {} ({} bytes)", label, self.active_rom.len()))
        };

        match load_result {
            Ok(msg) => {
                self.rom_path = Some(path);
                self.xdf.bind_from_rom(&self.active_rom, self.rom_origin);
                self.refresh_stock_tables();
                self.checksum_status = CHECKSUM_VALID.to_string();
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
                    self.selected_table_idx = 0;
                    self.active_cell = (0, 0);
                    let label = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                    self.xdf_path = Some(path);
                    self.xdf.bind_from_rom(&self.active_rom, self.rom_origin);
                    self.refresh_stock_tables();
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
        match PlatformSecurity::stamp_image(&prof.id, &mut image, self.rom_origin) {
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
        if self.selected_device_idx < self.devices.len() {
            let dev_info = &self.devices[self.selected_device_idx];
            let dev_name = dev_info.name.clone();
            let is_virtual = dev_info.is_virtual;
            let mut dev = create_device(dev_info);
            if let Ok(()) = dev.open() {
                if is_virtual {
                    PlatformSecurity::register_bench_keys_for_virtual();
                    self.log("Virtual bench: explicit zero-mask security keys registered for loopback only.");
                }
                let prof = &self.profiles[self.selected_profile_idx];
                match connect_for_profile(dev.as_mut(), prof) {
                    Ok(ch) => self.log(&format!("PassThru channel {} open ({})", ch, prof.id)),
                    Err(e) => {
                        self.log(&format!("Channel setup failed: {}", e));
                        return;
                    }
                }
                let ver = dev.firmware_version();
                self.log(&format!("Connected to {}: {}", dev_name, ver));
                self.active_device = Some(dev);
                self.is_connected = true;
            } else {
                self.log(&format!("Failed to open device: {}", dev_info.name));
            }
        }
    }

    pub fn disconnect_cable(&mut self) {
        if let Some(mut dev) = self.active_device.take() {
            let _ = dev.close();
            self.is_connected = false;
            self.log("Cable disconnected.");
        }
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
            let prof = &self.profiles[self.selected_profile_idx];
            let due: Vec<(usize, Vec<u8>, String)> = {
                let mut buf = self.telemetry.lock().unwrap();
                buf.scheduler
                    .due_indices(&buf.channels, now)
                    .into_iter()
                    .map(|idx| {
                        let ch = &buf.channels[idx];
                        (
                            idx,
                            ch.build_request_bytes(),
                            ch.decode.clone(),
                        )
                    })
                    .collect()
            };

            for (idx, req, decode_id) in due {
                if let Ok(()) =
                    crate::protocols::IsoTpLink::send(dev.as_mut(), 1, prof.request_id, &req)
                {
                    if let Ok(resp) = crate::protocols::IsoTpLink::receive(dev.as_mut(), 1, 10) {
                        if let Some(value) = decode_channel_value(&decode_id, &resp) {
                            let mut buf = self.telemetry.lock().unwrap();
                            buf.set_channel_value(idx, value);
                            buf.scheduler.mark_polled(idx, now);
                        }
                    }
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
}

impl eframe::App for WidebandApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_telemetry();
        ctx.request_repaint();

        theme::apply_motorsport_theme(ctx);

        egui::TopBottomPanel::top("header_bar")
            .exact_height(46.0)
            .frame(theme::header_frame())
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    paint_studio_badge(ui);
                    ui.add_space(14.0);

                    ui.label(egui::RichText::new("Vehicle").size(11.0).color(MUTED));
                    egui::ComboBox::from_id_salt("profile_select")
                        .width(200.0)
                        .selected_text(&self.profiles[self.selected_profile_idx].name)
                        .show_ui(ui, |ui| {
                            for (idx, prof) in self.profiles.iter().enumerate() {
                                ui.selectable_value(&mut self.selected_profile_idx, idx, &prof.name);
                            }
                        });

                    ui.add_space(10.0);

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
                            });
                        });

                    ui.add_space(8.0);

                    let batt_v = if self.is_connected { 13.4 } else { 0.0 };
                    let batt_label = if self.is_connected {
                        format!("{:.1}V", batt_v)
                    } else {
                        "—".to_string()
                    };
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
                                        .color(ACCENT_CYAN),
                                );
                            });
                        });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(6.0);
                        let tabs = [
                            (WorkspaceTab::Switchboard, "Switchboard"),
                            (WorkspaceTab::FlashDiagnostics, "Flash & Diag"),
                            (WorkspaceTab::Telemetry, "Telemetry"),
                            (WorkspaceTab::Calibration, "Calibration"),
                        ];
                        for (tab, title) in tabs {
                            if studio_tab_button(ui, title, self.active_tab == tab).clicked() {
                                self.active_tab = tab;
                            }
                            ui.add_space(6.0);
                        }

                        ui.add_space(12.0);
                        if self.is_connected {
                            if ui
                                .button(egui::RichText::new("Disconnect").size(11.0).color(MUTED))
                                .clicked()
                            {
                                self.disconnect_cable();
                            }
                        } else if ui
                            .button(egui::RichText::new("Connect").size(11.0).strong().color(ACCENT_CYAN))
                            .clicked()
                        {
                            self.connect_cable();
                        }
                        ui.add_space(8.0);
                        egui::ComboBox::from_id_salt("device_select")
                            .width(160.0)
                            .selected_text(
                                egui::RichText::new(&self.devices[self.selected_device_idx].name)
                                    .size(11.0)
                                    .color(MUTED),
                            )
                            .show_ui(ui, |ui| {
                                for (idx, dev) in self.devices.iter().enumerate() {
                                    ui.selectable_value(&mut self.selected_device_idx, idx, &dev.name);
                                }
                            });
                    });
                });
            });

        // -------------------------------------------------------------------
        // Workspace View Rendering
        // -------------------------------------------------------------------
        match self.active_tab {
            WorkspaceTab::Calibration => self.render_calibration_tab(ctx),
            WorkspaceTab::Telemetry => self.render_telemetry_tab(ctx),
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
        // Left Parameter Tree
        egui::SidePanel::left("param_tree")
            .resizable(true)
            .default_width(260.0)
            .width_range(200.0..=350.0)
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
                                        .hint_text("Search parameters...")
                                        .frame(false)
                                        .desired_width(ui.available_width());
                                    ui.add(edit);
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

                                for (idx, table) in self.xdf.tables.iter().enumerate() {
                                    if !self.search_query.is_empty()
                                        && !table
                                            .title
                                            .to_lowercase()
                                            .contains(&self.search_query.to_lowercase())
                                    {
                                        continue;
                                    }

                                    let is_selected = self.selected_table_idx == idx;
                                    let badge = if table.y_axis.dim > 1 { "[3D]" } else { "[2D]" };

                                    ui.allocate_ui(Vec2::new(ui.available_width(), 30.0), |ui| {
                                        ui.horizontal_centered(|ui| {
                                            ui.add_space(2.0);
                                            ui.label(
                                                egui::RichText::new(badge).strong().color(ACCENT_CYAN),
                                            );
                                            let title_color = if is_selected { TEXT } else { MUTED };
                                            if ui
                                                .selectable_label(
                                                    is_selected,
                                                    egui::RichText::new(&table.title).color(title_color),
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
                                for c in &self.xdf.constants {
                                    ui.allocate_ui(Vec2::new(ui.available_width(), 28.0), |ui| {
                                        ui.horizontal_centered(|ui| {
                                            ui.add_space(2.0);
                                            ui.label(
                                                egui::RichText::new("[1D]")
                                                    .strong()
                                                    .color(Color32::from_rgb(255, 193, 7)),
                                            );
                                            ui.label(
                                                egui::RichText::new(format!(
                                                    "{}: {:.0} {}",
                                                    c.title, c.value, c.units
                                                ))
                                                .color(TEXT),
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
            .default_width(260.0)
            .width_range(200.0..=350.0)
            .frame(theme::panel_frame())
            .show(ctx, |ui| {
                ui.heading(egui::RichText::new("3D Surface Mesh").size(14.0).color(ACCENT_CYAN));
                ui.label(egui::RichText::new("Drag to rotate pitch & yaw").small().color(MUTED));
                ui.add_space(4.0);

                let avail = ui.available_size();
                let telemetry_reserve = 128.0;
                let mesh_size = Vec2::new(
                    avail.x.max(120.0),
                    (avail.y - telemetry_reserve).max(140.0),
                );
                let (response, painter) =
                    ui.allocate_painter(mesh_size, egui::Sense::drag());
                if response.dragged() {
                    self.mesh_yaw += response.drag_delta().x * 0.01;
                    self.mesh_pitch = (self.mesh_pitch + response.drag_delta().y * 0.01).clamp(0.1, 1.4);
                }

                if !self.xdf.tables.is_empty() {
                    let table = &self.xdf.tables[self.selected_table_idx];
                    surface_mesh::paint_surface_mesh(
                        &painter,
                        response.rect,
                        table,
                        self.mesh_yaw,
                        self.mesh_pitch,
                        self.active_cell,
                    );
                } else {
                    painter.rect_filled(response.rect, 4.0, theme::BG_FILL);
                }

                ui.add_space(8.0);

                // Telemetry card styling: #11151C background, 1px #30363D border, bright cyan #00E5FF numbers
                egui::Frame::none()
                    .fill(CARD_FILL)
                    .stroke(Stroke::new(1.0, STROKE))
                    .rounding(egui::Rounding::same(4.0))
                    .inner_margin(egui::Margin::symmetric(12.0, 10.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("LIVE CELL TELEMETRY TRACKING")
                                    .small()
                                    .strong()
                                    .color(MUTED),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(egui::RichText::new("● LIVE").small().strong().color(EMERALD));
                            });
                        });
                        ui.add_space(6.0);

                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("Engine RPM:").color(MUTED));
                            ui.label(
                                egui::RichText::new(format!("{:.0}", self.trace_rpm))
                                    .monospace()
                                    .strong()
                                    .color(ACCENT_CYAN),
                            );
                            ui.add_space(10.0);
                            ui.label(egui::RichText::new("MAP:").color(MUTED));
                            ui.label(
                                egui::RichText::new(format!("{:.1} kPa", self.trace_map))
                                    .monospace()
                                    .strong()
                                    .color(ACCENT_CYAN),
                            );
                        });
                        ui.add_space(4.0);

                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("Active Tracked Cell:").color(MUTED));
                            ui.label(
                                egui::RichText::new(format!("Row {}, Col {}", self.active_cell.0, self.active_cell.1))
                                    .monospace()
                                    .strong()
                                    .color(ACCENT_CYAN),
                            );
                        });
                    });
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

                ui.horizontal(|ui| {
                    ui.heading(egui::RichText::new(&table_title).size(16.0).color(TEXT));
                    ui.label(egui::RichText::new(format!("({})", z_units)).color(ACCENT_CYAN));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let ck_color = if self.checksum_status.contains("Valid") {
                            EMERALD
                        } else {
                            Color32::from_rgb(0xF5, 0x88, 0x36)
                        };
                        ui.label(egui::RichText::new(&self.checksum_status).size(11.0).color(ck_color).strong());
                    });
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

                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::ScrollArea::both()
                            .id_salt("tune_grid_inner")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                self.render_table_grid(ui);
                            });
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
                            let is_delta = (val - stock_val).abs() > 0.01;
                            let is_tracked = self.active_cell == (r, c);

                            let bg_color = heatmap_color(val, min_val, max_val);
                            let text_color = Color32::from_rgb(0xFF, 0xFF, 0xFF);

                            row.col(|ui| {
                                let rect = ui.available_rect_before_wrap();
                                let painter = ui.painter();

                                painter.rect_filled(rect, 2.0, bg_color);

                                if is_tracked {
                                    paint_live_cell_glow(painter, rect);
                                }

                                // If delta from stock: draw small bold gold indicator
                                if is_delta {
                                    let delta_pos = Pos2::new(rect.right() - 6.0, rect.top() + 4.0);
                                    painter.circle_filled(delta_pos, 2.5, Color32::from_rgb(255, 214, 0));
                                }

                                let response = ui.selectable_label(
                                    is_tracked,
                                    egui::RichText::new(format!("{:>6.1}", val)).monospace().strong().color(text_color),
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
        let (gauge_channels, recording, def_title) = {
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
            (gauges, buf.is_recording, buf.definition_title.clone())
        };

        egui::TopBottomPanel::top("gauge_strip").min_height(72.0).show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                ui.add_space(8.0);
                for (label, text, unit, rgb) in &gauge_channels {
                    self.render_gauge_card(
                        ui,
                        label,
                        text.clone(),
                        unit,
                        Color32::from_rgb(rgb[0], rgb[1], rgb[2]),
                    );
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Export CSV Log").clicked() {
                        let exported = self.telemetry.lock().unwrap().export_csv("datalog.csv").is_ok();
                        if exported {
                            self.log("Datalog exported successfully to datalog.csv");
                        } else {
                            self.log("Export failed — start live polling or record first.");
                        }
                    }
                    let rec_label = if recording { "■ Stop Record" } else { "● Record" };
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
                    if ui.button("Clear Log").clicked() {
                        self.telemetry.lock().unwrap().clear();
                        self.log("Telemetry history cleared.");
                    }
                });
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
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
            ui.add_space(4.0);

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
                .view_aspect(2.2)
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

// ---------------------------------------------------------------------------
// Tab 3: Diagnostics & ECU Flashing
// ---------------------------------------------------------------------------

impl WidebandApp {
    fn render_flash_tab(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading(egui::RichText::new("ECU Diagnostics & Laboratory Bench-Flashing").size(16.0).color(Color32::WHITE));
            ui.label("Direct J2534 ISO 14229 / ISO 14230 flashing with seed-key handshake and checksum stamping.");
            ui.separator();

            ui.horizontal(|ui| {
                if ui.button(egui::RichText::new("🔍 Identify ECU").strong()).clicked() {
                    if let Some(dev) = &mut self.active_device {
                        let prof = &self.profiles[self.selected_profile_idx];
                        match EcuIdentifier::query(dev.as_mut(), 1, prof) {
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

                if ui.button(egui::RichText::new("⚡ Flash Active Calibration").strong().color(Color32::from_rgb(255, 23, 68))).clicked() {
                    self.execute_flash_active();
                }
            });

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

            ui.add_space(8.0);
            ui.heading("Communication & Flash Event Log");
            egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                for line in &self.comm_log {
                    ui.label(egui::RichText::new(line).monospace().small());
                }
            });
        });
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
            let read_res = FlashJob::read_stock(
                dev.as_mut(),
                channel,
                &prof,
                &self.flash_layout,
                &label,
                &self.flash_audit,
                |_done, _total| {},
            );
            match read_res {
                Ok(stock) => {
                    self.active_rom = stock.bytes;
                    self.rom_origin = stock.origin;
                    self.xdf.bind_from_rom(&self.active_rom, self.rom_origin);
                    self.stock_tables = self.xdf.tables.iter().map(|t| t.values.clone()).collect();
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

        self.log("Stamping and verifying checksum before cal-only flash...");
        let prof = self.profiles[self.selected_profile_idx].clone();
        self.xdf.write_all_tables_to_rom(&mut self.active_rom, self.rom_origin);
        let prepared = match PlatformSecurity::prepare_for_flash(
            &prof.id,
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
                                let flag = &self.xdf.flags[idx];
                                self.xdf.write_flag_to_rom(&mut self.active_rom, self.rom_origin, flag);
                                self.checksum_status = CHECKSUM_UPDATED.to_string();
                            }

                            let flag = &self.xdf.flags[idx];
                            let badge_color = if flag.is_set {
                                Color32::from_rgb(0, 230, 118)
                            } else {
                                Color32::from_rgb(140, 150, 165)
                            };
                            let status_lbl = if flag.is_set { "[ENABLED]" } else { "[DISABLED]" };
                            ui.label(egui::RichText::new(status_lbl).strong().color(badge_color));

                            ui.vertical(|ui| {
                                ui.label(egui::RichText::new(&flag.title).strong().color(Color32::WHITE));
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
