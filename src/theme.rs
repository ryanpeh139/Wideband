//! Haltech NSP / MoTeC M1 Tune–inspired dark motorsport studio palette for egui.

use eframe::egui::{self, Color32, Rounding, Stroke, Visuals};

// ---------------------------------------------------------------------------
// Design tokens (exact hex from product spec)
// ---------------------------------------------------------------------------

pub const BG_FILL: Color32 = Color32::from_rgb(0x0D, 0x11, 0x17);
pub const PANEL_FILL: Color32 = Color32::from_rgb(0x16, 0x1B, 0x22);
pub const WIDGET_BG: Color32 = Color32::from_rgb(0x21, 0x26, 0x2D);
pub const STROKE: Color32 = Color32::from_rgb(0x30, 0x36, 0x3D);
pub const TEXT: Color32 = Color32::from_rgb(0xE6, 0xED, 0xF3);
pub const MUTED: Color32 = Color32::from_rgb(0x8B, 0x94, 0x9E);
pub const ACCENT_BLUE: Color32 = Color32::from_rgb(0x2F, 0x81, 0xF7);
pub const ACCENT_CYAN: Color32 = Color32::from_rgb(0x00, 0xE5, 0xFF);
pub const HOVER_FILL: Color32 = Color32::from_rgb(0x30, 0x36, 0x3D);
pub const EMERALD: Color32 = Color32::from_rgb(0x2E, 0xDA, 0x7A);
pub const TABLE_HEADER: Color32 = Color32::from_rgb(0x1C, 0x21, 0x28);
pub const CARD_FILL: Color32 = Color32::from_rgb(0x11, 0x15, 0x1C);

pub const ROUNDING: f32 = 4.0;

fn round_all() -> Rounding {
    Rounding::same(ROUNDING)
}

/// Apply the motorsport studio theme to the egui context (startup + each frame).
pub fn apply_motorsport_theme(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals = motorsport_visuals();
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 7.0);
    style.spacing.indent = 18.0;
    style.spacing.interact_size = egui::vec2(40.0, 24.0);
    ctx.set_style(style);
}

pub fn motorsport_visuals() -> Visuals {
    let mut visuals = Visuals::dark();
    let r = round_all();

    visuals.override_text_color = Some(TEXT);
    visuals.extreme_bg_color = BG_FILL;
    visuals.faint_bg_color = WIDGET_BG;
    visuals.window_fill = PANEL_FILL;
    visuals.panel_fill = PANEL_FILL;
    visuals.window_stroke = Stroke::new(1.0, STROKE);
    visuals.window_shadow = egui::epaint::Shadow::NONE;
    visuals.window_rounding = r;
    visuals.menu_rounding = r;

    visuals.widgets.noninteractive.bg_fill = WIDGET_BG;
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, MUTED);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, STROKE);
    visuals.widgets.noninteractive.rounding = r;

    visuals.widgets.inactive.bg_fill = WIDGET_BG;
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, STROKE);
    visuals.widgets.inactive.rounding = r;

    visuals.widgets.hovered.bg_fill = HOVER_FILL;
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, MUTED);
    visuals.widgets.hovered.rounding = r;

    visuals.widgets.active.bg_fill = ACCENT_BLUE;
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.active.bg_stroke = Stroke::new(1.5, ACCENT_CYAN);
    visuals.widgets.active.rounding = r;

    visuals.widgets.open.bg_fill = WIDGET_BG;
    visuals.widgets.open.fg_stroke = Stroke::new(1.0, ACCENT_CYAN);
    visuals.widgets.open.bg_stroke = Stroke::new(1.5, ACCENT_BLUE);
    visuals.widgets.open.rounding = r;

    visuals.selection.bg_fill = ACCENT_BLUE.gamma_multiply(0.35);
    visuals.selection.stroke = Stroke::new(1.0, ACCENT_CYAN);

    visuals.hyperlink_color = ACCENT_CYAN;
    visuals.warn_fg_color = Color32::from_rgb(0xF5, 0x88, 0x36);
    visuals.error_fg_color = Color32::from_rgb(0xF8, 0x51, 0x49);

    visuals
}

/// Standard panel chrome (1px #30363D border, studio panel fill).
pub fn panel_frame() -> egui::Frame {
    egui::Frame::none()
        .fill(PANEL_FILL)
        .stroke(Stroke::new(1.0, STROKE))
        .inner_margin(egui::Margin::symmetric(10.0, 8.0))
        .rounding(round_all())
}

pub fn header_frame() -> egui::Frame {
    egui::Frame::none()
        .fill(PANEL_FILL)
        .stroke(Stroke::new(1.0, STROKE))
        .inner_margin(egui::Margin::symmetric(12.0, 6.0))
}

/// Owner-drawn studio brand badge: `WIDEBAND // MOTORSPORT STUDIO`.
pub fn paint_studio_badge(ui: &mut egui::Ui) {
    let height = 28.0;
    let (rect, _response) = ui.allocate_exact_size(egui::vec2(280.0, height), egui::Sense::hover());
    let painter = ui.painter();

    painter.rect_filled(rect, ROUNDING, WIDGET_BG);
    painter.rect_stroke(rect, ROUNDING, Stroke::new(1.0, STROKE));

    let accent_bar = egui::Rect::from_min_size(rect.min, egui::vec2(3.0, rect.height()));
    painter.rect_filled(accent_bar, 0.0, ACCENT_CYAN);

    let text_y = rect.center().y;
    let mut x = rect.min.x + 10.0;

    let wide = painter.layout_no_wrap(
        "WIDEBAND".into(),
        egui::FontId::proportional(13.0),
        TEXT,
    );
    let wide_w = wide.size().x;
    painter.galley(egui::pos2(x, text_y - wide.size().y * 0.5), wide, TEXT);
    x += wide_w + 6.0;

    let sep = painter.layout_no_wrap("//".into(), egui::FontId::monospace(11.0), MUTED);
    let sep_w = sep.size().x;
    painter.galley(egui::pos2(x, text_y - sep.size().y * 0.5), sep, MUTED);
    x += sep_w + 6.0;

    let studio = painter.layout_no_wrap(
        "MOTORSPORT STUDIO".into(),
        egui::FontId::proportional(11.0),
        MUTED,
    );
    painter.galley(egui::pos2(x, text_y - studio.size().y * 0.5), studio, MUTED);
}

/// Cobalt → Teal → Warm Amber → Crimson heatmap ramp.
pub fn heatmap_color(val: f64, min: f64, max: f64) -> Color32 {
    let norm = if max > min {
        ((val - min) / (max - min)).clamp(0.0, 1.0)
    } else {
        0.5
    };

    const COBALT: (u8, u8, u8) = (0x1A, 0x3A, 0x8C);
    const TEAL: (u8, u8, u8) = (0x0D, 0x94, 0x88);
    const AMBER: (u8, u8, u8) = (0xF5, 0x9E, 0x0B);
    const CRIMSON: (u8, u8, u8) = (0xB9, 0x1C, 0x1C);

    let lerp = |a: u8, b: u8, t: f64| -> u8 {
        (a as f64 + (b as f64 - a as f64) * t).round() as u8
    };
    let mix = |c1: (u8, u8, u8), c2: (u8, u8, u8), t: f64| -> Color32 {
        Color32::from_rgb(
            lerp(c1.0, c2.0, t),
            lerp(c1.1, c2.1, t),
            lerp(c1.2, c2.2, t),
        )
    };

    if norm < 0.33 {
        mix(COBALT, TEAL, norm / 0.33)
    } else if norm < 0.66 {
        mix(TEAL, AMBER, (norm - 0.33) / 0.33)
    } else {
        mix(AMBER, CRIMSON, (norm - 0.66) / 0.34)
    }
}

/// 1px neon cyan glow around the live-traced RPM/MAP cell.
pub fn paint_live_cell_glow(painter: &egui::Painter, rect: egui::Rect) {
    let cyan = ACCENT_CYAN;
    painter.rect_stroke(rect.shrink(0.5), 2.0, Stroke::new(1.0, cyan));
    painter.rect_stroke(rect.expand(1.0), 3.0, Stroke::new(1.0, cyan.gamma_multiply(0.45)));
}

pub fn studio_tab_button(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    let text = if selected {
        egui::RichText::new(label)
            .size(12.0)
            .strong()
            .color(ACCENT_CYAN)
    } else {
        egui::RichText::new(label).size(12.0).color(MUTED)
    };

    let mut button = egui::Button::new(text).min_size(egui::vec2(96.0, 28.0));
    if selected {
        button = button.fill(WIDGET_BG).stroke(Stroke::new(1.5, ACCENT_BLUE));
    } else {
        button = button.fill(Color32::TRANSPARENT).stroke(Stroke::new(1.0, STROKE));
    }
    ui.add(button)
}
