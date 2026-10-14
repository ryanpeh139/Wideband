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

/// System fonts with arrows / geometric shapes (→ ← ● ▶ ⏸ ↻) that egui's bundled Ubuntu font
/// lacks — without one those glyphs render as empty boxes. First existing file wins.
const SYMBOL_FALLBACK_FONTS: &[&str] = &[
    "C:\\Windows\\Fonts\\seguisym.ttf",
    "C:\\Windows\\Fonts\\segoeui.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
];

/// Append an installed symbol font as the last fallback of both families. Call once at startup
/// (`set_fonts` re-rasterises the atlas). No-op when none is installed.
pub fn install_symbol_fallback_font(ctx: &egui::Context) {
    let Some((path, bytes)) = SYMBOL_FALLBACK_FONTS
        .iter()
        .find_map(|p| std::fs::read(p).ok().map(|b| (*p, b)))
    else {
        eprintln!("theme: no system symbol font found; arrows/bullets may render as boxes");
        return;
    };
    let mut fonts = egui::FontDefinitions::default();
    fonts
        .font_data
        .insert("symbol-fallback".to_owned(), egui::FontData::from_owned(bytes));
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push("symbol-fallback".to_owned());
    }
    ctx.set_fonts(fonts);
    eprintln!("theme: symbol fallback font {}", path);
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum StudioHeaderTier {
    Wide,
    Medium,
    Compact,
}

pub fn studio_header_tier(ctx: &egui::Context) -> StudioHeaderTier {
    let w = ctx.screen_rect().width();
    if w < 920.0 {
        StudioHeaderTier::Compact
    } else if w < 1260.0 {
        StudioHeaderTier::Medium
    } else {
        StudioHeaderTier::Wide
    }
}

/// Proportional side-panel width (default, min, max) from the current viewport.
pub fn side_panel_widths(ctx: &egui::Context) -> (f32, f32, f32) {
    let w = ctx.screen_rect().width().max(520.0);
    let default_w = (w * 0.20).clamp(170.0, 300.0);
    let min_w = 150.0;
    // Each side panel capped at ~26% so two of them always leave the centre (table grid)
    // about half the window; widths remembered from a larger window are clamped to this.
    let max_w = (w * 0.26).clamp(170.0, 360.0);
    (default_w, min_w, max_w)
}

/// Owner-drawn studio brand badge: `WIDEBAND // MOTORSPORT STUDIO`.
pub fn paint_studio_badge(ui: &mut egui::Ui, compact: bool) {
    let height = 28.0;
    let width = if compact { 196.0 } else { 280.0 };
    let (rect, _response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
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

    if !compact {
        let studio = painter.layout_no_wrap(
            "MOTORSPORT STUDIO".into(),
            egui::FontId::proportional(11.0),
            MUTED,
        );
        painter.galley(egui::pos2(x, text_y - studio.size().y * 0.5), studio, MUTED);
    }
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

/// Live operating-point cell (engine is here now) — green, distinct from the cyan edit selection.
pub fn paint_operating_cell_glow(painter: &egui::Painter, rect: egui::Rect) {
    painter.rect_stroke(rect.shrink(1.5), 2.0, Stroke::new(2.0, EMERALD));
    painter.rect_stroke(rect.expand(2.0), 3.0, Stroke::new(1.5, EMERALD.gamma_multiply(0.4)));
}

pub fn studio_tab_button(
    ui: &mut egui::Ui,
    label: &str,
    selected: bool,
    compact: bool,
) -> egui::Response {
    let font_size = if compact { 11.0 } else { 12.0 };
    let text = if selected {
        egui::RichText::new(label)
            .size(font_size)
            .strong()
            .color(ACCENT_CYAN)
    } else {
        egui::RichText::new(label).size(font_size).color(MUTED)
    };

    let min_w = if compact { 76.0 } else { 96.0 };
    let mut button = egui::Button::new(text).min_size(egui::vec2(min_w, 28.0));
    if selected {
        button = button.fill(WIDGET_BG).stroke(Stroke::new(1.5, ACCENT_BLUE));
    } else {
        button = button.fill(Color32::TRANSPARENT).stroke(Stroke::new(1.0, STROKE));
    }
    ui.add(button)
}
