use crate::managers::ManagerRegistry;
use crate::patch_recipe::PatchRecipe;
use crate::strategy_meta::{build_strategy_graph, ParamKind, StrategyRole};
use crate::theme::{ACCENT_CYAN, CARD_FILL, MUTED, STROKE, TEXT};
use crate::ui::{WidebandApp, WorkspaceTab};
use eframe::egui::{self, Color32, Stroke, Vec2};
use std::path::PathBuf;

impl WidebandApp {
    pub fn render_strategy_tab(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(crate::theme::panel_frame())
            .show(ctx, |ui| {
                ui.heading(
                    egui::RichText::new("Strategy Map & Managers")
                        .color(Color32::WHITE),
                );
                ui.label(
                    egui::RichText::new(
                        "Graph from plain-text wb-meta: tags in XDF descriptions (no encrypted definitions).",
                    )
                    .small()
                    .color(MUTED),
                );
                ui.add_space(6.0);

                ui.horizontal(|ui| {
                    let graph = build_strategy_graph(&self.xdf);
                    ui.vertical(|ui| {
                        ui.set_min_width(ui.available_width() * 0.55);
                        self.paint_strategy_map(ui, &graph);
                    });
                    ui.separator();
                    ui.vertical(|ui| {
                        self.render_manager_panels(ui);
                        ui.add_space(8.0);
                        ui.separator();
                        self.render_patch_recipes(ui);
                    });
                });
            });
    }

    fn paint_strategy_map(
        &mut self,
        ui: &mut egui::Ui,
        graph: &crate::strategy_meta::StrategyGraph,
    ) {
        let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 280.0), egui::Sense::click());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 6.0, CARD_FILL);
        painter.rect_stroke(rect, 6.0, Stroke::new(1.0, STROKE));

        if graph.nodes.is_empty() {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Load an XDF with wb-meta: tags to populate the strategy map.",
                egui::FontId::proportional(13.0),
                MUTED,
            );
            return;
        }

        let pad = 28.0;
        let inner = rect.shrink(pad);
        let node_positions: std::collections::HashMap<String, egui::Pos2> = graph
            .nodes
            .iter()
            .map(|n| {
                let (nx, ny) = graph.positions.get(&n.id).copied().unwrap_or((0.5, 0.5));
                let pos = egui::pos2(
                    inner.left() + nx * inner.width(),
                    inner.top() + ny * inner.height(),
                );
                (n.id.clone(), pos)
            })
            .collect();

        for edge in &graph.edges {
            if let (Some(&from), Some(&to)) =
                (node_positions.get(&edge.from_id), node_positions.get(&edge.to_id))
            {
                painter.line_segment(
                    [from, to],
                    Stroke::new(1.5, ACCENT_CYAN.gamma_multiply(0.55)),
                );
                let dir = (to - from).normalized();
                let tip = to - dir * 14.0;
                painter.circle_filled(to, 3.0, ACCENT_CYAN);
                painter.line_segment([tip, to], Stroke::new(2.0, ACCENT_CYAN));
            }
        }

        for node in &graph.nodes {
            let center = node_positions[&node.id];
            let size = Vec2::new(118.0, 36.0);
            let nrect = egui::Rect::from_center_size(center, size);
            let role_color = role_color(node.meta.role);
            painter.rect_filled(nrect, 4.0, role_color.gamma_multiply(0.35));
            painter.rect_stroke(nrect, 4.0, Stroke::new(1.0, role_color));
            painter.text(
                nrect.center(),
                egui::Align2::CENTER_CENTER,
                truncate(&node.title, 16),
                egui::FontId::proportional(11.0),
                TEXT,
            );
        }

        if response.clicked() {
            let click = response.interact_pointer_pos().unwrap_or(rect.center());
            for node in &graph.nodes {
                let center = node_positions[&node.id];
                if center.distance(click) < 24.0 {
                    self.jump_to_strategy_node(node);
                }
            }
        }
    }

    fn jump_to_strategy_node(&mut self, node: &crate::strategy_meta::StrategyNode) {
        match node.kind {
            ParamKind::Table => {
                if let Some(idx) = node.table_idx {
                    self.selected_table_idx = idx;
                    self.active_tab = WorkspaceTab::Calibration;
                }
            }
            ParamKind::Constant | ParamKind::Flag => {
                self.active_tab = WorkspaceTab::Switchboard;
            }
        }
    }

    fn render_manager_panels(&mut self, ui: &mut egui::Ui) {
        ui.heading(egui::RichText::new("Managers").size(14.0).color(MUTED));
        let registry = ManagerRegistry::load_or_embedded();
        let panels = registry.panels_for_xdf(&self.xdf);
        if panels.is_empty() {
            ui.label("No manager registry.");
            return;
        }
        let idx = self.strategy_manager_idx.min(panels.len() - 1);
        ui.horizontal(|ui| {
            for (i, panel) in panels.iter().enumerate() {
                let selected = i == idx;
                if ui
                    .selectable_label(selected, &panel.definition.title)
                    .clicked()
                {
                    self.strategy_manager_idx = i;
                }
            }
        });
        let panel = &panels[idx];
        ui.label(
            egui::RichText::new(&panel.definition.description)
                .small()
                .color(MUTED),
        );
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .max_height(160.0)
            .show(ui, |ui| {
                if panel.members.is_empty() {
                    ui.label("No parameters matched this manager for the active XDF.");
                }
                for member in &panel.members {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(member.meta.role.label()).monospace().small());
                        if ui.button(&member.title).clicked() {
                            self.jump_to_strategy_node(member);
                        }
                    });
                }
            });
    }

    fn render_patch_recipes(&mut self, ui: &mut egui::Ui) {
        ui.heading(egui::RichText::new("EROM-style patch recipes").size(14.0).color(MUTED));
        ui.label(
            egui::RichText::new("Declarative JSON/YAML only — preview byte diff before apply.")
                .small()
                .color(MUTED),
        );
        ui.horizontal(|ui| {
            if ui.button("Load recipe…").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Patch recipe", &["json", "yaml", "yml"])
                    .pick_file()
                {
                    self.load_patch_recipe(path);
                }
            }
            if ui.button("Load sample recipe").clicked() {
                self.load_patch_recipe(PathBuf::from("sample-patch-recipe.json"));
            }
            if self.loaded_patch.is_some() {
                if ui.button("Preview diff").clicked() {
                    self.run_patch_preview();
                }
                if ui.button("Apply to ROM").clicked() {
                    self.apply_patch_to_rom();
                }
            }
        });
        if let Some(recipe) = &self.loaded_patch {
            ui.label(format!("Loaded: {} ({})", recipe.file.title, recipe.file.id));
        }
        if !self.patch_status.is_empty() {
            ui.colored_label(Color32::LIGHT_GREEN, &self.patch_status);
        }
        if let Some(preview) = &self.patch_preview {
            ui.separator();
            ui.label(format!(
                "Preview: {} byte(s) would change",
                preview.diffs.len()
            ));
            egui::ScrollArea::vertical()
                .max_height(120.0)
                .show(ui, |ui| {
                    for diff in preview.diffs.iter().take(80) {
                        ui.monospace(format!(
                            "0x{:06X}  {:02X} → {:02X}",
                            diff.address,
                            diff.old_byte,
                            diff.new_byte
                        ));
                    }
                    if preview.diffs.len() > 80 {
                        ui.label(format!("… +{} more", preview.diffs.len() - 80));
                    }
                });
        }
    }

    pub fn load_patch_recipe(&mut self, path: PathBuf) {
        match PatchRecipe::load(&path) {
            Ok(recipe) => {
                let label = path.display().to_string();
                self.loaded_patch = Some(recipe);
                self.patch_recipe_path = Some(path);
                self.patch_preview = None;
                self.patch_status = format!("Recipe loaded from {}", label);
            }
            Err(e) => {
                self.patch_status = format!("Recipe load failed: {}", e);
            }
        }
    }

    pub fn run_patch_preview(&mut self) {
        if let Some(recipe) = &self.loaded_patch {
            match recipe.preview(&self.active_rom, self.rom_origin, &self.xdf) {
                Ok(preview) => {
                    self.patch_preview = Some(preview);
                    self.patch_status = "Preview ready — review diff before Apply or flash.".to_string();
                }
                Err(e) => self.patch_status = format!("Preview failed: {}", e),
            }
        }
    }

    pub fn apply_patch_to_rom(&mut self) {
        if let Some(recipe) = self.loaded_patch.clone() {
            match recipe.apply(&mut self.active_rom, self.rom_origin, &mut self.xdf) {
                Ok(notes) => {
                    self.refresh_checksum_badge();
                    self.patch_preview = None;
                    self.patch_status = format!("Applied: {}", notes.join("; "));
                    self.log(&format!("Patch applied: {}", recipe.file.id));
                }
                Err(e) => self.patch_status = format!("Apply failed: {}", e),
            }
        }
    }
}

fn role_color(role: StrategyRole) -> Color32 {
    match role {
        StrategyRole::Limiter => Color32::from_rgb(220, 80, 80),
        StrategyRole::Target => Color32::from_rgb(80, 180, 255),
        StrategyRole::Actuator => Color32::from_rgb(120, 220, 140),
        StrategyRole::Sensor => Color32::from_rgb(200, 200, 100),
        StrategyRole::Logic | StrategyRole::Flag => Color32::from_rgb(180, 120, 255),
        StrategyRole::Constant => Color32::from_rgb(160, 160, 200),
        StrategyRole::Table => Color32::from_rgb(100, 140, 180),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>())
    }
}
