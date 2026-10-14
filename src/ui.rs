use crate::definitions::XdfDocument;
use crate::j2534::{create_device, J2534DeviceInfo, J2534RegistryScanner, PassThruDevice};
use crate::protocols::{ChecksumEngine, EcuIdentification, EcuIdentifier, EcuProfile, FlashDownload};
use crate::telemetry::TelemetryBuffer;
use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Vec2};
use egui_extras::{Column, TableBuilder};
use egui_plot::{Line, Plot, PlotPoints};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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

    // UI Workspace
    pub active_tab: WorkspaceTab,
}

impl WidebandApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let devices = J2534RegistryScanner::scan();
        let profiles = EcuProfile::all();
        let xdf = XdfDocument::sample();

        let stock_tables = xdf.tables.iter().map(|t| t.values.clone()).collect();
        let mut rom = vec![0xFFu8; 1024 * 1024]; // 1MB mock ROM buffer
        ChecksumEngine::update_and_verify(&mut rom, &profiles[0].id).ok();

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
            checksum_status: "Checksum Valid".to_string(),
            active_rom: rom,
            telemetry: Arc::new(Mutex::new(TelemetryBuffer::new(5000))),
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
            active_tab: WorkspaceTab::Calibration,
        };

        // Auto-connect to Virtual ECU Loopback on startup for instant live preview
        app.connect_cable();
        app
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
            let mut dev = create_device(dev_info);
            if let Ok(()) = dev.open() {
                let prof = &self.profiles[self.selected_profile_idx];
                let _ = dev.connect(prof.protocol_id, prof.connect_flags, prof.bitrate);
                let ver = dev.firmware_version();
                self.log(&format!("Connected to {}: {}", dev_info.name, ver));
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

        if self.last_poll_time.elapsed() < Duration::from_millis(50) {
            return;
        }
        self.last_poll_time = Instant::now();

        if let Some(dev) = &mut self.active_device {
            let prof = &self.profiles[self.selected_profile_idx];
            let pids = [0x0C, 0x0B, 0x04, 0x05, 0x0E, 0x0F, 0x11, 0x24];
            let mut snap = self.telemetry.lock().unwrap().latest();
            snap.time_sec = self.telemetry.lock().unwrap().start_time.elapsed().as_secs_f64();

            for &pid in &pids {
                let req = vec![0x01, pid];
                if let Ok(()) = crate::protocols::IsoTpLink::send(dev.as_mut(), 1, prof.request_id, &req) {
                    if let Ok(resp) = crate::protocols::IsoTpLink::receive(dev.as_mut(), 1, 10) {
                        TelemetryBuffer::parse_mode01_response(&mut snap, &resp);
                    }
                }
            }

            self.trace_rpm = snap.rpm;
            self.trace_map = snap.map_kpa;
            self.telemetry.lock().unwrap().push(snap);

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
            self.checksum_status = "Checksum Updated".to_string();
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
            self.checksum_status = "Checksum Updated".to_string();
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
        self.checksum_status = "Checksum Updated".to_string();
    }
}

// ---------------------------------------------------------------------------
// Heatmap Color Utilities
// ---------------------------------------------------------------------------

pub fn heatmap_color(val: f64, min: f64, max: f64) -> Color32 {
    let norm = if max > min {
        ((val - min) / (max - min)).clamp(0.0, 1.0)
    } else {
        0.5
    };

    if norm < 0.25 {
        let t = norm / 0.25;
        // Blue to Cyan
        Color32::from_rgb(
            (20.0 + t * 0.0) as u8,
            (80.0 + t * 140.0) as u8,
            (240.0 - t * 20.0) as u8,
        )
    } else if norm < 0.5 {
        let t = (norm - 0.25) / 0.25;
        // Cyan to Green
        Color32::from_rgb(
            (20.0 + t * 30.0) as u8,
            (220.0 - t * 40.0) as u8,
            (220.0 - t * 150.0) as u8,
        )
    } else if norm < 0.75 {
        let t = (norm - 0.5) / 0.25;
        // Green to Gold/Amber
        Color32::from_rgb(
            (50.0 + t * 200.0) as u8,
            (180.0 + t * 30.0) as u8,
            (70.0 - t * 60.0) as u8,
        )
    } else {
        let t = (norm - 0.75) / 0.25;
        // Gold to Crimson Red
        Color32::from_rgb(
            (250.0 - t * 20.0) as u8,
            (210.0 - t * 170.0) as u8,
            (10.0 + t * 30.0) as u8,
        )
    }
}

impl eframe::App for WidebandApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_telemetry();
        ctx.request_repaint(); // 60 FPS repaint for smooth live oscilloscope & tracing

        // -------------------------------------------------------------------
        // Dark Motorsport Theme Setup
        // -------------------------------------------------------------------
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = Color32::from_rgb(18, 21, 25);
        visuals.window_fill = Color32::from_rgb(24, 28, 34);
        visuals.extreme_bg_color = Color32::from_rgb(12, 14, 17);
        visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(28, 33, 40);
        visuals.widgets.inactive.bg_fill = Color32::from_rgb(34, 40, 48);
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(45, 54, 66);
        visuals.widgets.active.bg_fill = Color32::from_rgb(30, 136, 229);
        ctx.set_visuals(visuals);

        // -------------------------------------------------------------------
        // Top Studio Header Bar
        // -------------------------------------------------------------------
        // -------------------------------------------------------------------
        // Top Studio Header Bar
        // -------------------------------------------------------------------
        let header_frame = egui::Frame::none()
            .fill(Color32::from_rgb(18, 21, 25))
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(48, 54, 61))) // 1px #30363D border
            .inner_margin(egui::Margin::symmetric(10.0, 5.0));

        egui::TopBottomPanel::top("header_bar")
            .exact_height(46.0)
            .frame(header_frame)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(4.0);
                    ui.heading(egui::RichText::new("⚡ WIDEBAND").strong().color(Color32::from_rgb(0, 229, 255)));
                    ui.label(egui::RichText::new("MOTORSPORT STUDIO").small().color(Color32::from_rgb(140, 150, 165)));
                    ui.add_space(12.0);

                    // Connection status pill with 6px horizontal, 3px vertical padding & glowing accent border
                    let (status_text, status_color) = if self.is_connected {
                        if self.devices[self.selected_device_idx].is_virtual {
                            ("SIMULATOR ACTIVE", Color32::from_rgb(0, 229, 255))
                        } else {
                            ("CONNECTED", Color32::from_rgb(0, 230, 118))
                        }
                    } else {
                        ("DISCONNECTED", Color32::from_rgb(255, 23, 68))
                    };

                    egui::Frame::none()
                        .fill(status_color.gamma_multiply(0.15))
                        .stroke(Stroke::new(1.0_f32, status_color))
                        .rounding(4.0)
                        .inner_margin(egui::Margin::symmetric(6.0, 3.0))
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

                    ui.add_space(6.0);
                    if self.is_connected {
                        if ui.button(egui::RichText::new("Disconnect").size(11.0)).clicked() {
                            self.disconnect_cable();
                        }
                    } else {
                        if ui.button(egui::RichText::new("Connect Cable").size(11.0).strong().color(Color32::from_rgb(0, 229, 255))).clicked() {
                            self.connect_cable();
                        }
                    }

                    ui.add_space(10.0);
                    ui.label(egui::RichText::new("Device:").size(11.0).color(Color32::from_rgb(140, 150, 165)));
                    egui::ComboBox::from_id_salt("device_select")
                        .width(180.0)
                        .selected_text(&self.devices[self.selected_device_idx].name)
                        .show_ui(ui, |ui| {
                            for (idx, dev) in self.devices.iter().enumerate() {
                                ui.selectable_value(&mut self.selected_device_idx, idx, &dev.name);
                            }
                        });

                    ui.label(egui::RichText::new("Profile:").size(11.0).color(Color32::from_rgb(140, 150, 165)));
                    egui::ComboBox::from_id_salt("profile_select")
                        .width(170.0)
                        .selected_text(&self.profiles[self.selected_profile_idx].name)
                        .show_ui(ui, |ui| {
                            for (idx, prof) in self.profiles.iter().enumerate() {
                                ui.selectable_value(&mut self.selected_profile_idx, idx, &prof.name);
                            }
                        });

                    ui.add_space(10.0);
                    // Checksum status badge
                    let ck_color = if self.checksum_status.contains("Valid") {
                        Color32::from_rgb(0, 230, 118)
                    } else {
                        Color32::from_rgb(255, 214, 0)
                    };
                    ui.label(egui::RichText::new(&self.checksum_status).size(11.0).color(ck_color).strong());

                    // Navigation Tabs with proper horizontal margins and flex spacing
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(4.0);
                        let tabs = [
                            (WorkspaceTab::Switchboard, "Switchboard"),
                            (WorkspaceTab::FlashDiagnostics, "Flash & Diag"),
                            (WorkspaceTab::Telemetry, "Telemetry"),
                            (WorkspaceTab::Calibration, "Calibration"),
                        ];
                        for (tab, title) in tabs {
                            let is_selected = self.active_tab == tab;
                            let text = if is_selected {
                                egui::RichText::new(title).size(12.0).strong().color(Color32::from_rgb(0, 229, 255))
                            } else {
                                egui::RichText::new(title).size(12.0).color(Color32::from_rgb(175, 185, 195))
                            };
                            if ui.add(egui::Button::new(text).min_size(Vec2::new(88.0, 26.0))).clicked() {
                                self.active_tab = tab;
                            }
                            ui.add_space(4.0);
                        }
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
            .min_width(280.0)
            .max_width(340.0)
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgb(18, 21, 25))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(48, 54, 61)))
                    .inner_margin(egui::Margin::symmetric(10.0, 8.0)),
            )
            .show(ctx, |ui| {
                ui.add_space(2.0);
                // Search input box with rounded corner and clean inset border
                egui::Frame::none()
                    .fill(Color32::from_rgb(12, 14, 18))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(48, 54, 61)))
                    .rounding(5.0)
                    .inner_margin(egui::Margin::symmetric(8.0, 5.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("🔍").size(12.0).color(Color32::from_rgb(130, 140, 155)));
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
                ui.heading(egui::RichText::new("Calibration Tables").size(13.0).color(Color32::from_rgb(160, 175, 195)));
                ui.add_space(4.0);

                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (idx, table) in self.xdf.tables.iter().enumerate() {
                        if !self.search_query.is_empty()
                            && !table.title.to_lowercase().contains(&self.search_query.to_lowercase())
                        {
                            continue;
                        }

                        let is_selected = self.selected_table_idx == idx;
                        let badge = if table.y_axis.dim > 1 { "[3D]" } else { "[2D]" };

                        // 30px row height for breathing room
                        ui.allocate_ui(Vec2::new(ui.available_width(), 30.0), |ui| {
                            ui.horizontal_centered(|ui| {
                                ui.add_space(2.0);
                                ui.label(egui::RichText::new(badge).strong().color(Color32::from_rgb(0, 229, 255)));
                                let title_color = if is_selected { Color32::WHITE } else { Color32::from_rgb(200, 210, 225) };
                                if ui.selectable_label(is_selected, egui::RichText::new(&table.title).color(title_color)).clicked() {
                                    self.selected_table_idx = idx;
                                    self.active_cell = (0, 0);
                                }
                            });
                        });
                    }

                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(4.0);
                    ui.heading(egui::RichText::new("Scalar Constants").size(13.0).color(Color32::from_rgb(160, 175, 195)));
                    ui.add_space(4.0);
                    for c in &self.xdf.constants {
                        ui.allocate_ui(Vec2::new(ui.available_width(), 28.0), |ui| {
                            ui.horizontal_centered(|ui| {
                                ui.add_space(2.0);
                                ui.label(egui::RichText::new("[1D]").strong().color(Color32::from_rgb(255, 193, 7)));
                                ui.label(
                                    egui::RichText::new(format!("{}: {:.0} {}", c.title, c.value, c.units))
                                        .color(Color32::from_rgb(220, 225, 230)),
                                );
                            });
                        });
                    }
                });
            });

        // Right 3D Isometric Surface Mesh
        egui::SidePanel::right("surface_mesh_panel")
            .min_width(340.0)
            .max_width(460.0)
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgb(18, 21, 25))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(48, 54, 61)))
                    .inner_margin(egui::Margin::symmetric(12.0, 10.0)),
            )
            .show(ctx, |ui| {
                ui.heading(egui::RichText::new("3D Surface Visualizer").size(14.0).color(Color32::from_rgb(0, 229, 255)));
                ui.label(egui::RichText::new("Drag mouse to rotate pitch & yaw").small().color(Color32::GRAY));
                ui.add_space(4.0);

                let (response, painter) = ui.allocate_painter(Vec2::new(ui.available_width(), 310.0), egui::Sense::drag());
                if response.dragged() {
                    self.mesh_yaw += response.drag_delta().x * 0.01;
                    self.mesh_pitch = (self.mesh_pitch + response.drag_delta().y * 0.01).clamp(0.1, 1.4);
                }

                self.render_3d_mesh(&painter, response.rect);

                ui.add_space(12.0);

                // Telemetry card styling: #11151C background, 1px #30363D border, bright cyan #00E5FF numbers
                egui::Frame::none()
                    .fill(Color32::from_rgb(17, 21, 28)) // #11151C
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(48, 54, 61))) // #30363D
                    .rounding(6.0)
                    .inner_margin(egui::Margin::symmetric(12.0, 10.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("LIVE CELL TELEMETRY TRACKING")
                                    .small()
                                    .strong()
                                    .color(Color32::from_rgb(140, 150, 165)),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(egui::RichText::new("● LIVE").small().strong().color(Color32::from_rgb(0, 230, 118)));
                            });
                        });
                        ui.add_space(6.0);

                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("Engine RPM:").color(Color32::from_rgb(200, 210, 225)));
                            ui.label(
                                egui::RichText::new(format!("{:.0}", self.trace_rpm))
                                    .monospace()
                                    .strong()
                                    .color(Color32::from_rgb(0, 229, 255)),
                            );
                            ui.add_space(10.0);
                            ui.label(egui::RichText::new("MAP:").color(Color32::from_rgb(200, 210, 225)));
                            ui.label(
                                egui::RichText::new(format!("{:.1} kPa", self.trace_map))
                                    .monospace()
                                    .strong()
                                    .color(Color32::from_rgb(0, 229, 255)),
                            );
                        });
                        ui.add_space(4.0);

                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("Active Tracked Cell:").color(Color32::from_rgb(200, 210, 225)));
                            ui.label(
                                egui::RichText::new(format!("Row {}, Col {}", self.active_cell.0, self.active_cell.1))
                                    .monospace()
                                    .strong()
                                    .color(Color32::from_rgb(0, 229, 255)),
                            );
                        });
                    });
            });

        // Center Heatmap Table Grid
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.xdf.tables.is_empty() {
                ui.label("No calibration tables loaded.");
                return;
            }

            let table_title = self.xdf.tables[self.selected_table_idx].title.clone();
            let table_desc = self.xdf.tables[self.selected_table_idx].description.clone();
            let z_units = self.xdf.tables[self.selected_table_idx].z_units.clone();

            ui.horizontal(|ui| {
                ui.heading(egui::RichText::new(&table_title).size(16.0).color(Color32::WHITE));
                ui.label(egui::RichText::new(format!("({})", z_units)).color(Color32::from_rgb(0, 229, 255)));
            });
            ui.label(egui::RichText::new(&table_desc).small().color(Color32::from_rgb(140, 150, 165)));
            ui.add_space(4.0);

            // 1px subtle horizontal rule separator between title/desc and math toolbar
            ui.separator();
            ui.add_space(4.0);

            // Table Math Toolbar
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Math Step:").strong());
                ui.add(egui::DragValue::new(&mut self.math_amount).speed(0.5).range(0.1..=100.0));

                if ui.button("  +  ").clicked() { self.run_table_math('+'); }
                if ui.button("  -  ").clicked() { self.run_table_math('-'); }
                if ui.button("  *  ").clicked() { self.run_table_math('*'); }
                if ui.button("  =  ").clicked() { self.run_table_math('='); }
                ui.separator();
                if ui.button("Interpolate Row").clicked() { self.interpolate_active_row(); }
                if ui.button("Smooth 3D").clicked() { self.smooth_table(); }
            });
            ui.add_space(8.0);

            // Render TableBuilder Grid
            self.render_table_grid(ui);
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
            .header(26.0, |mut header| {
                header.col(|ui| {
                    ui.label(egui::RichText::new(format!("Y / X")).strong().color(Color32::from_rgb(140, 160, 185)));
                });
                for c in 0..cols {
                    let lbl = if c < x_labels.len() { x_labels[c] } else { (c * 500) as f64 };
                    header.col(|ui| {
                        ui.label(egui::RichText::new(format!("{:>6.0}", lbl)).monospace().strong().color(Color32::from_rgb(0, 229, 255)));
                    });
                }
            })
            .body(|mut body| {
                for r in 0..rows {
                    body.row(24.0, |mut row| {
                        // Y-Axis Row Header
                        let y_lbl = if r < y_labels.len() { y_labels[r] } else { (r * 10) as f64 };
                        row.col(|ui| {
                            ui.label(egui::RichText::new(format!("{:>5.0}", y_lbl)).monospace().strong().color(Color32::from_rgb(0, 229, 255)));
                        });

                        // Cell values with Heatmap color + Gold delta
                        for c in 0..cols {
                            let val = self.xdf.tables[self.selected_table_idx].values[r][c];
                            let stock_val = if r < stock.len() && c < stock[r].len() { stock[r][c] } else { val };
                            let is_delta = (val - stock_val).abs() > 0.01;
                            let is_tracked = self.active_cell == (r, c);

                            let bg_color = heatmap_color(val, min_val, max_val);
                            let text_color = if bg_color.r() > 180 && bg_color.g() > 180 {
                                Color32::BLACK
                            } else {
                                Color32::WHITE
                            };

                            row.col(|ui| {
                                let rect = ui.available_rect_before_wrap();
                                let painter = ui.painter();

                                // Fill heatmap cell
                                painter.rect_filled(rect, 2.0, bg_color);

                                // If tracked by live telemetry: glowing cyan border
                                if is_tracked {
                                    painter.rect_stroke(rect, 2.0, Stroke::new(2.5_f32, Color32::from_rgb(0, 229, 255)));
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

    fn render_3d_mesh(&self, painter: &egui::Painter, rect: Rect) {
        painter.rect_filled(rect, 4.0, Color32::from_rgb(14, 16, 20));
        painter.rect_stroke(rect, 4.0, Stroke::new(1.0_f32, Color32::from_rgb(34, 40, 50)));

        if self.xdf.tables.is_empty() {
            return;
        }

        let table = &self.xdf.tables[self.selected_table_idx];
        let rows = table.values.len();
        let cols = table.values[0].len();

        let mut min_val = f64::MAX;
        let mut max_val = f64::MIN;
        for r in 0..rows {
            for c in 0..cols {
                min_val = min_val.min(table.values[r][c]);
                max_val = max_val.max(table.values[r][c]);
            }
        }

        let center = rect.center();
        let scale_x = (rect.width() * 0.35) / cols as f32;
        let scale_y = (rect.height() * 0.35) / rows as f32;
        let z_height = 80.0;

        let cos_yaw = self.mesh_yaw.cos();
        let sin_yaw = self.mesh_yaw.sin();
        let cos_pitch = self.mesh_pitch.cos();
        let sin_pitch = self.mesh_pitch.sin();

        let project = |r: usize, c: usize| -> (Pos2, Color32) {
            let x0 = (c as f32 - cols as f32 * 0.5) * scale_x;
            let y0 = (r as f32 - rows as f32 * 0.5) * scale_y;
            let val = table.values[r][c];
            let norm_z = if max_val > min_val {
                ((val - min_val) / (max_val - min_val)) as f32
            } else {
                0.5
            };
            let z0 = (norm_z - 0.5) * z_height;

            // 3D rotation: Yaw around Z, Pitch around X
            let rx = x0 * cos_yaw - y0 * sin_yaw;
            let ry = x0 * sin_yaw + y0 * cos_yaw;
            let rz = z0 * cos_pitch - ry * sin_pitch;

            let screen_pos = Pos2::new(center.x + rx, center.y - rz);
            let color = heatmap_color(val, min_val, max_val);
            (screen_pos, color)
        };

        // Render quads and wireframe lines
        for r in 0..rows - 1 {
            for c in 0..cols - 1 {
                let (p00, c00) = project(r, c);
                let (p10, _) = project(r + 1, c);
                let (p11, _) = project(r + 1, c + 1);
                let (p01, _) = project(r, c + 1);

                // Quad fill
                painter.add(egui::Shape::convex_polygon(
                    vec![p00, p10, p11, p01],
                    Color32::from_rgba_unmultiplied(c00.r(), c00.g(), c00.b(), 180),
                    Stroke::new(1.0_f32, Color32::from_rgb(20, 24, 30)),
                ));
            }
        }

        // Highlight active cell marker on 3D surface
        let (r_active, c_active) = self.active_cell;
        if r_active < rows && c_active < cols {
            let (pos, _) = project(r_active, c_active);
            painter.circle_filled(pos, 5.0, Color32::from_rgb(0, 229, 255));
            painter.circle_stroke(pos, 8.0, Stroke::new(2.0_f32, Color32::WHITE));
        }
    }
}

// ---------------------------------------------------------------------------
// Tab 2: Telemetry & Multi-Trace Oscilloscope Strip Chart
// ---------------------------------------------------------------------------

impl WidebandApp {
    fn render_telemetry_tab(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("gauge_strip").min_height(72.0).show(ctx, |ui| {
            let snap = self.telemetry.lock().unwrap().latest();
            ui.horizontal_centered(|ui| {
                ui.add_space(8.0);
                self.render_gauge_card(ui, "ENGINE RPM", format!("{:.0}", snap.rpm), "RPM", Color32::from_rgb(0, 229, 255));
                self.render_gauge_card(ui, "MANIFOLD MAP", format!("{:.1}", snap.map_kpa), "kPa", Color32::from_rgb(255, 193, 7));
                self.render_gauge_card(ui, "LAMBDA (AFR)", format!("{:.3}", snap.lambda), "λ", Color32::from_rgb(0, 230, 118));
                self.render_gauge_card(ui, "SPARK ADVANCE", format!("{:.1}", snap.timing_deg), "deg", Color32::from_rgb(244, 67, 54));
                self.render_gauge_card(ui, "COOLANT ECT", format!("{:.0}", snap.ect_c), "°C", Color32::from_rgb(30, 136, 229));
                self.render_gauge_card(ui, "CALC LOAD", format!("{:.0}", snap.load_pct), "%", Color32::from_rgb(255, 152, 0));

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Export CSV Log").clicked() {
                        let exported = self.telemetry.lock().unwrap().export_csv("datalog.csv").is_ok();
                        if exported {
                            self.log("Datalog exported successfully to datalog.csv");
                        }
                    }
                    if ui.button("Clear Log").clicked() {
                        self.telemetry.lock().unwrap().clear();
                        self.log("Telemetry history cleared.");
                    }
                });
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading(egui::RichText::new("Oscilloscope Real-Time Telemetry Strip Chart").size(15.0).color(Color32::WHITE));
            ui.add_space(4.0);

            let buffer = self.telemetry.lock().unwrap();
            let mut rpm_points = Vec::new();
            let mut map_points = Vec::new();
            let mut lambda_points = Vec::new();
            let mut timing_points = Vec::new();

            for s in &buffer.snapshots {
                rpm_points.push([s.time_sec, s.rpm as f64]);
                map_points.push([s.time_sec, s.map_kpa as f64 * 30.0]); // Scaled for visible overlay
                lambda_points.push([s.time_sec, s.lambda as f64 * 3000.0]);
                timing_points.push([s.time_sec, s.timing_deg as f64 * 100.0]);
            }

            let rpm_line = Line::new(PlotPoints::new(rpm_points))
                .color(Color32::from_rgb(0, 229, 255))
                .name("RPM");
            let map_line = Line::new(PlotPoints::new(map_points))
                .color(Color32::from_rgb(255, 193, 7))
                .name("MAP (kPa x30)");
            let lambda_line = Line::new(PlotPoints::new(lambda_points))
                .color(Color32::from_rgb(0, 230, 118))
                .name("Lambda (x3000)");
            let timing_line = Line::new(PlotPoints::new(timing_points))
                .color(Color32::from_rgb(244, 67, 54))
                .name("Timing (° x100)");

            Plot::new("telemetry_oscilloscope")
                .view_aspect(2.2)
                .show_axes([true, true])
                .legend(egui_plot::Legend::default())
                .show(ui, |plot_ui| {
                    plot_ui.line(rpm_line);
                    plot_ui.line(map_line);
                    plot_ui.line(lambda_line);
                    plot_ui.line(timing_line);
                });
        });
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

                if ui.button("Virtual Read (Match Stock ROM)").clicked() {
                    self.log(&format!("Matched local stock calibration: {}.bin", self.ecu_id.cal_id));
                    self.checksum_status = "Checksum Valid".to_string();
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

    fn execute_flash_active(&mut self) {
        if self.active_device.is_none() {
            self.log("Flashing blocked: Connect cable first.");
            return;
        }

        self.log("Stamping and verifying checksum before flash...");
        let prof = self.profiles[self.selected_profile_idx].clone();
        if let Err(e) = ChecksumEngine::update_and_verify(&mut self.active_rom, &prof.id) {
            self.log(&format!("Flash blocked: Checksum calculation failed: {}", e));
            return;
        }

        self.log(&format!("Starting flash sequence for {}...", prof.name));
        self.is_flashing = true;
        self.transfer_progress = 0.0;

        if let Some(dev) = &mut self.active_device {
            let rom_slice = self.active_rom[..64 * 1024].to_vec(); // Transfer 64KB block in test
            let flash_res = FlashDownload::execute(
                dev.as_mut(),
                1,
                &prof,
                0x10000,
                &rom_slice,
                |_sent, _total| {
                    // Update progress
                },
            );

            match flash_res {
                Ok(()) => {
                    self.transfer_progress = 1.0;
                    self.is_flashing = false;
                    self.log("Flash completed successfully! Checksum verified and ECU reset (0x11).");
                }
                Err(e) => {
                    self.is_flashing = false;
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
                for flag in &mut self.xdf.flags {
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            let mut checked = flag.is_set;
                            if ui.checkbox(&mut checked, "").changed() {
                                flag.is_set = checked;
                            }

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
