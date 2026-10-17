//! Simple path UX: beginner mode, first-run wizard, and virtual practice ladder.

use crate::theme::{self, ACCENT_CYAN, CARD_FILL, EMERALD, MUTED, STROKE, TEXT};
use crate::ui::{BeginnerStep, WidebandApp, WorkspaceTab};
use eframe::egui::{self, Color32, Stroke};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkillLevel {
    Beginner,
    Advanced,
}

impl Default for SkillLevel {
    fn default() -> Self {
        SkillLevel::Beginner
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WizardStep {
    Welcome,
    CableDriver,
    VehicleProfile,
    ConnectCheck,
    Ready,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VirtualLadderState {
    Idle,
    Running {
        index: usize,
        started: Instant,
        step_started: Instant,
    },
    Done,
    Failed(String),
}

fn beginner_step_ord(step: BeginnerStep) -> u8 {
    match step {
        BeginnerStep::Connect => 0,
        BeginnerStep::Identify => 1,
        BeginnerStep::Log => 2,
        BeginnerStep::Calibrate => 3,
    }
}

const LADDER_STEP_LABELS: [&str; 4] = [
    "Connect Virtual ECU (bench)",
    "Identify ECU",
    "Live telemetry",
    "Read-only DTCs & readiness",
];

/// Tables/constants exposed in beginner cal step (safe subset).
pub fn is_beginner_safe_table(id: &str) -> bool {
    matches!(id, "target_lambda" | "ve_main")
}

pub fn is_beginner_safe_constant(id: &str) -> bool {
    id == "rev_limit"
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct UxPrefsFile {
    first_run_complete: bool,
    skill_level: SkillLevel,
}

impl Default for UxPrefsFile {
    fn default() -> Self {
        Self {
            first_run_complete: false,
            skill_level: SkillLevel::Beginner,
        }
    }
}

pub struct SimpleUxState {
    pub skill_level: SkillLevel,
    pub first_run_complete: bool,
    pub wizard_open: bool,
    pub wizard_step: WizardStep,
    pub beginner_step: BeginnerStep,
    pub virtual_ladder: VirtualLadderState,
    pub ladder_message: String,
    prefs_path: PathBuf,
    dirty: bool,
}

impl SimpleUxState {
    pub fn load() -> Self {
        let fallback = Self::user_prefs_path();
        let path = match &fallback {
            Some(p) if p.exists() => p.clone(),
            _ => Self::prefs_path(),
        };
        let file: UxPrefsFile = if path.exists() {
            fs::read_to_string(&path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default()
        } else {
            UxPrefsFile::default()
        };

        let wizard_open = !file.first_run_complete;
        Self {
            skill_level: file.skill_level,
            first_run_complete: file.first_run_complete,
            wizard_open,
            wizard_step: WizardStep::Welcome,
            beginner_step: BeginnerStep::Connect,
            virtual_ladder: VirtualLadderState::Idle,
            ladder_message: String::new(),
            prefs_path: path,
            dirty: false,
        }
    }

    pub fn prefs_path() -> PathBuf {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                return dir.join("wideband-ux-prefs.json");
            }
        }
        PathBuf::from("wideband-ux-prefs.json")
    }

    /// Per-user copy, used when the program folder is not writable (for example Program Files).
    fn user_prefs_path() -> Option<PathBuf> {
        std::env::var_os("LOCALAPPDATA")
            .or_else(|| std::env::var_os("APPDATA"))
            .map(|dir| PathBuf::from(dir).join("Wideband").join("wideband-ux-prefs.json"))
    }

    pub fn is_beginner(&self) -> bool {
        self.skill_level == SkillLevel::Beginner
    }

    pub fn simplified_chrome(&self) -> bool {
        self.is_beginner() || self.wizard_open
    }

    pub fn persist_if_dirty(&mut self) {
        if !self.dirty {
            return;
        }
        let body = UxPrefsFile {
            first_run_complete: self.first_run_complete,
            skill_level: self.skill_level,
        };
        if let Ok(json) = serde_json::to_string_pretty(&body) {
            if fs::write(&self.prefs_path, &json).is_err() {
                if let Some(user_path) = Self::user_prefs_path() {
                    if let Some(dir) = user_path.parent() {
                        let _ = fs::create_dir_all(dir);
                    }
                    if fs::write(&user_path, &json).is_ok() {
                        self.prefs_path = user_path;
                    }
                }
            }
            self.dirty = false;
        }
    }

    fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn set_skill_level(&mut self, level: SkillLevel) {
        if self.skill_level != level {
            self.skill_level = level;
            self.mark_dirty();
        }
    }

    pub fn complete_first_run(&mut self) {
        self.first_run_complete = true;
        self.wizard_open = false;
        self.mark_dirty();
    }
}

impl WidebandApp {
    pub fn profile_index_for_id(&self, profile_id: &str) -> Option<usize> {
        self.profiles
            .iter()
            .position(|p| p.id.eq_ignore_ascii_case(profile_id))
    }

    pub fn select_virtual_device(&mut self) {
        if let Some(idx) = self.devices.iter().position(|d| d.is_virtual_bench()) {
            self.selected_device_idx = idx;
        }
    }

    pub fn select_profile_id(&mut self, profile_id: &str) {
        if let Some(idx) = self.profile_index_for_id(profile_id) {
            self.selected_profile_idx = idx;
        }
    }

    pub fn start_virtual_practice_ladder(&mut self) {
        self.simple_ux.ladder_message.clear();
        self.simple_ux.virtual_ladder = VirtualLadderState::Running {
            index: 0,
            started: Instant::now(),
            step_started: Instant::now(),
        };
        self.log("Virtual practice ladder: starting bench-only walkthrough.");
        if self.is_connected {
            self.disconnect_cable();
        }
        self.select_virtual_device();
        self.select_profile_id("tacoma-can");
        self.simple_ux.beginner_step = BeginnerStep::Connect;
        self.simple_ux.skill_level = SkillLevel::Beginner;
    }

    pub fn tick_virtual_ladder(&mut self) {
        let state = self.simple_ux.virtual_ladder.clone();
        match state {
            VirtualLadderState::Idle | VirtualLadderState::Done | VirtualLadderState::Failed(_) => {}
            VirtualLadderState::Running {
                index,
                started,
                step_started,
            } => {
                let step_timeout = Duration::from_secs(12);
                if step_started.elapsed() > step_timeout {
                    self.simple_ux.virtual_ladder = VirtualLadderState::Failed(format!(
                        "Timed out on step: {}",
                        LADDER_STEP_LABELS
                            .get(index)
                            .unwrap_or(&"unknown")
                    ));
                    self.log("Virtual practice ladder failed (timeout).");
                    return;
                }

                match index {
                    0 => {
                        if !self.is_connected {
                            self.connect_cable();
                        } else {
                            self.simple_ux.virtual_ladder = VirtualLadderState::Running {
                                index: 1,
                                started,
                                step_started: Instant::now(),
                            };
                        }
                    }
                    1 => {
                        if self.ecu_id.vin.is_empty() {
                            self.run_identify_ecu();
                        } else {
                            self.simple_ux.beginner_step = BeginnerStep::Identify;
                            self.simple_ux.virtual_ladder = VirtualLadderState::Running {
                                index: 2,
                                started,
                                step_started: Instant::now(),
                            };
                        }
                    }
                    2 => {
                        self.live_poller_active = true;
                        self.active_tab = WorkspaceTab::Telemetry;
                        self.simple_ux.beginner_step = BeginnerStep::Log;
                        if step_started.elapsed() >= Duration::from_millis(800) {
                            self.simple_ux.virtual_ladder = VirtualLadderState::Running {
                                index: 3,
                                started,
                                step_started: Instant::now(),
                            };
                        }
                    }
                    3 => {
                        if self.diag_snapshot.stored_dtcs.is_empty()
                            && self.diag_last_error.is_empty()
                        {
                            self.refresh_diagnostics();
                        }
                        if !self.diag_snapshot.stored_dtcs.is_empty()
                            || !self.diag_last_error.is_empty()
                            || step_started.elapsed() >= Duration::from_secs(2)
                        {
                            self.simple_ux.beginner_step = BeginnerStep::Log;
                            self.simple_ux.virtual_ladder = VirtualLadderState::Done;
                            self.simple_ux.ladder_message = format!(
                                "Practice complete in {:.1}s — identified {}, telemetry live, read-only diagnostics sampled. Connect a real J2534 cable when ready.",
                                started.elapsed().as_secs_f32(),
                                if self.ecu_id.vin.is_empty() {
                                    "ECU"
                                } else {
                                    &self.ecu_id.vin
                                }
                            );
                            self.log("Virtual practice ladder complete (bench only).");
                        }
                    }
                    _ => {
                        self.simple_ux.virtual_ladder = VirtualLadderState::Done;
                    }
                }
            }
        }
    }

    pub fn run_identify_ecu(&mut self) {
        let allow_demo = self.cable_is_virtual();
        if let Some(dev) = &mut self.active_device {
            let prof = &self.profiles[self.selected_profile_idx];
            match crate::protocols::EcuIdentifier::query(dev.as_mut(), 1, prof, allow_demo) {
                Ok(id) => {
                    self.ecu_id = id;
                    self.log(&format!(
                        "ECU Identified: VIN={}, Cal ID={}, CVN={}",
                        self.ecu_id.vin,
                        self.ecu_id.cal_id,
                        self.ecu_id.cvn
                    ));
                }
                Err(e) => self.log(&format!("ECU identification failed: {}", e)),
            }
        } else {
            self.log("Connect cable first.");
        }
    }

    pub fn render_ux_overlays(&mut self, ctx: &egui::Context) {
        self.simple_ux.persist_if_dirty();
        if self.simple_ux.wizard_open {
            self.render_first_run_wizard(ctx);
        }
    }

    pub fn render_skill_and_practice_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Experience").size(11.0).color(MUTED));
            let beginner = self.simple_ux.skill_level == SkillLevel::Beginner;
            if ui
                .selectable_label(beginner, egui::RichText::new("Beginner").strong())
                .clicked()
            {
                self.simple_ux.set_skill_level(SkillLevel::Beginner);
            }
            if ui
                .selectable_label(!beginner, egui::RichText::new("Advanced").strong())
                .clicked()
            {
                self.simple_ux.set_skill_level(SkillLevel::Advanced);
            }
            ui.separator();
            if ui
                .button(
                    egui::RichText::new("Virtual ECU practice")
                        .color(ACCENT_CYAN)
                        .strong(),
                )
                .on_hover_text(
                    "Bench-only loopback: identify → telemetry → read-only DTCs before using a real cable.",
                )
                .clicked()
            {
                self.start_virtual_practice_ladder();
            }
            if !self.simple_ux.ladder_message.is_empty() {
                ui.label(
                    egui::RichText::new(&self.simple_ux.ladder_message)
                        .small()
                        .color(EMERALD),
                );
            }
            match &self.simple_ux.virtual_ladder {
                VirtualLadderState::Running { index, .. } => {
                    let label = LADDER_STEP_LABELS
                        .get(*index)
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| "Working…".to_string());
                    ui.label(
                        egui::RichText::new(format!("Ladder: {}", label))
                            .small()
                            .color(Color32::from_rgb(255, 193, 7)),
                    );
                }
                VirtualLadderState::Failed(msg) => {
                    ui.label(
                        egui::RichText::new(format!("Ladder failed: {}", msg))
                            .small()
                            .color(Color32::from_rgb(255, 82, 82)),
                    );
                }
                _ => {}
            }
        });
    }

    pub fn render_beginner_path_strip(&mut self, ui: &mut egui::Ui) {
        let steps = [
            (BeginnerStep::Connect, "1 · Connect"),
            (BeginnerStep::Identify, "2 · Identify"),
            (BeginnerStep::Log, "3 · Log"),
            (BeginnerStep::Calibrate, "4 · Cal"),
        ];
        ui.horizontal_wrapped(|ui| {
            for (step, label) in steps {
                let active = self.simple_ux.beginner_step == step;
                let done = beginner_step_ord(step) < beginner_step_ord(self.simple_ux.beginner_step);
                let color = if active {
                    ACCENT_CYAN
                } else if done {
                    EMERALD
                } else {
                    MUTED
                };
                if ui
                    .selectable_label(active, egui::RichText::new(label).strong().color(color))
                    .clicked()
                {
                    self.simple_ux.beginner_step = step;
                    self.sync_beginner_step_to_tab();
                }
                ui.add_space(6.0);
            }
        });
    }

    pub(crate) fn sync_beginner_step_to_tab(&mut self) {
        self.active_tab = match self.simple_ux.beginner_step {
            BeginnerStep::Connect | BeginnerStep::Identify => WorkspaceTab::FlashDiagnostics,
            BeginnerStep::Log => WorkspaceTab::Telemetry,
            BeginnerStep::Calibrate => WorkspaceTab::Calibration,
        };
    }

    pub fn render_beginner_workspace(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("beginner_path")
            .frame(theme::panel_frame())
            .show(ctx, |ui| {
                self.render_beginner_path_strip(ui);
            });

        match self.simple_ux.beginner_step {
            BeginnerStep::Connect => self.render_beginner_connect(ctx),
            BeginnerStep::Identify => self.render_beginner_identify(ctx),
            BeginnerStep::Log => self.render_telemetry_tab(ctx, true),
            BeginnerStep::Calibrate => self.render_calibration_tab(ctx, true),
        }
    }

    fn render_beginner_connect(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(theme::panel_frame())
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(24.0);
                    ui.heading(
                        egui::RichText::new("Connect your Pass-Thru cable")
                            .size(20.0)
                            .color(TEXT),
                    );
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(
                            "Pick your J2534 device and vehicle profile, then connect. Use Virtual ECU for bench practice first.",
                        )
                        .color(MUTED),
                    );
                    ui.add_space(20.0);
                    egui::Frame::none()
                        .fill(CARD_FILL)
                        .stroke(Stroke::new(1.0, STROKE))
                        .rounding(egui::Rounding::same(8.0))
                        .inner_margin(egui::Margin::symmetric(24.0, 20.0))
                        .show(ui, |ui| {
                            ui.set_max_width(520.0);
                            self.paint_profile_selector(ui, 280.0);
                            ui.add_space(12.0);
                            self.paint_connect_controls(ui, 280.0);
                            ui.add_space(12.0);
                            if self.is_connected {
                                if ui.button("Next: Identify ECU →").clicked() {
                                    self.simple_ux.beginner_step = BeginnerStep::Identify;
                                    self.sync_beginner_step_to_tab();
                                }
                            }
                        });
                });
            });
    }

    fn render_beginner_identify(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(theme::panel_frame())
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(24.0);
                    ui.heading(egui::RichText::new("Identify the ECU").size(20.0).color(TEXT));
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new("Read VIN, calibration ID, and CVN before changing maps.")
                            .color(MUTED),
                    );
                    ui.add_space(16.0);
                    if ui.button(egui::RichText::new("Identify ECU").strong()).clicked() {
                        self.run_identify_ecu();
                    }
                    ui.add_space(12.0);
                    egui::Frame::none()
                        .fill(CARD_FILL)
                        .stroke(Stroke::new(1.0, STROKE))
                        .inner_margin(egui::Margin::symmetric(16.0, 12.0))
                        .show(ui, |ui| {
                            ui.label(format!(
                                "VIN: {}",
                                if self.ecu_id.vin.is_empty() {
                                    "—"
                                } else {
                                    &self.ecu_id.vin
                                }
                            ));
                            ui.label(format!(
                                "Cal ID: {}",
                                if self.ecu_id.cal_id.is_empty() {
                                    "—"
                                } else {
                                    &self.ecu_id.cal_id
                                }
                            ));
                        });
                    if !self.ecu_id.vin.is_empty() {
                        ui.add_space(12.0);
                        if ui.button("Next: Live log →").clicked() {
                            self.simple_ux.beginner_step = BeginnerStep::Log;
                            self.sync_beginner_step_to_tab();
                        }
                    }
                });
            });
    }

    fn render_first_run_wizard(&mut self, ctx: &egui::Context) {
        egui::Window::new("Welcome to Motorsport Studio")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_width(480.0)
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new("One clear path: connect → identify → log → safe cal edits.")
                        .color(MUTED),
                );
                ui.add_space(12.0);

                match self.simple_ux.wizard_step {
                    WizardStep::Welcome => {
                        ui.label(
                            "Beginner mode hides advanced flash tables and switchboard until you switch to Advanced.",
                        );
                        ui.add_space(8.0);
                        if ui.button("Get started →").clicked() {
                            self.simple_ux.wizard_step = WizardStep::CableDriver;
                        }
                    }
                    WizardStep::CableDriver => {
                        ui.heading("Cable & driver");
                        ui.label("• Windows: install your adapter’s SAE J2534 Pass-Thru driver (Mini-VCI, VXDIAG Toyota, Tactrix, etc.).");
                        ui.label("• Place J2534Host.exe (x86) beside wideband.exe for 32-bit DLLs.");
                        ui.label("• Before truck day: run scripts\\Friday-CablePrep.ps1 (PASS/FAIL) and device_check.ps1 on your PassThru DLL.");
                        ui.label("• No cable yet? Use Virtual ECU practice from the header anytime.");
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ui.button("← Back").clicked() {
                                self.simple_ux.wizard_step = WizardStep::Welcome;
                            }
                            if ui.button("Next →").clicked() {
                                self.simple_ux.wizard_step = WizardStep::VehicleProfile;
                            }
                        });
                    }
                    WizardStep::VehicleProfile => {
                        ui.heading("Vehicle profile");
                        ui.label("Default: Toyota Tacoma 2009+ (Denso CAN). Change if your truck differs.");
                        self.select_profile_id("tacoma-can");
                        self.paint_profile_selector(ui, 300.0);
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ui.button("← Back").clicked() {
                                self.simple_ux.wizard_step = WizardStep::CableDriver;
                            }
                            if ui.button("Next →").clicked() {
                                self.simple_ux.wizard_step = WizardStep::ConnectCheck;
                                if !self.is_connected {
                                    self.selected_device_idx =
                                        crate::transport::TransportDevice::preferred_index(&self.devices);
                                }
                            }
                        });
                    }
                    WizardStep::ConnectCheck => {
                        ui.heading("Connection check");
                        self.paint_connect_controls(ui, 260.0);
                        ui.add_space(6.0);
                        if self.is_connected {
                            ui.label(egui::RichText::new("● Connected").color(EMERALD));
                        }
                        ui.horizontal(|ui| {
                            if ui.button("← Back").clicked() {
                                self.simple_ux.wizard_step = WizardStep::VehicleProfile;
                            }
                            if ui.button("Skip connect").clicked() {
                                self.simple_ux.wizard_step = WizardStep::Ready;
                            }
                            if self.is_connected && ui.button("Next →").clicked() {
                                self.simple_ux.wizard_step = WizardStep::Ready;
                            }
                        });
                    }
                    WizardStep::Ready => {
                        ui.heading("You're ready");
                        ui.label("Beginner path is enabled. Run Virtual ECU practice before your real cable.");
                        ui.add_space(8.0);
                        if ui.button("Start Motorsport Studio").clicked() {
                            self.simple_ux.complete_first_run();
                            self.simple_ux.set_skill_level(SkillLevel::Beginner);
                            self.simple_ux.beginner_step = BeginnerStep::Connect;
                            // Practice runs on the Virtual ECU only; a real cable connects when the user asks.
                            let virtual_selected = self
                                .devices
                                .get(self.selected_device_idx)
                                .map(|d| d.is_virtual_bench())
                                .unwrap_or(false);
                            if !self.is_connected && virtual_selected {
                                self.connect_cable();
                            }
                        }
                    }
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beginner_safe_subset_is_narrow() {
        assert!(is_beginner_safe_table("target_lambda"));
        assert!(is_beginner_safe_table("ve_main"));
        assert!(!is_beginner_safe_table("spark_base"));
        assert!(is_beginner_safe_constant("rev_limit"));
        assert!(!is_beginner_safe_constant("speed_limiter"));
    }

    #[test]
    fn ux_prefs_roundtrip() {
        let dir = std::env::temp_dir().join("wideband-ux-test");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("prefs.json");
        let body = UxPrefsFile {
            first_run_complete: true,
            skill_level: SkillLevel::Advanced,
        };
        fs::write(&path, serde_json::to_string(&body).unwrap()).unwrap();
        let read: UxPrefsFile =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert!(read.first_run_complete);
        assert_eq!(read.skill_level, SkillLevel::Advanced);
    }
}
