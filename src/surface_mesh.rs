//! Isometric 3D calibration surface mesh with painter's-algorithm depth sorting.
//!
//! Everything is sorted and shaded in camera (rotated) space: the camera sits on the -Y side
//! of rotated space looking toward +Y, screen X = rotated X, screen up = rotated Z.

use crate::definitions::XdfTable;
use crate::theme::{heatmap_color, ACCENT_CYAN, BG_FILL, EMERALD, MUTED, STROKE, TEXT};
use eframe::egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Shape, Stroke, Vec2};

/// Max screen distance (px) from a vertex for hover / click picking.
const PICK_RADIUS: f32 = 14.0;

#[derive(Clone, Copy)]
struct Vec3 {
    x: f32,
    y: f32,
    z: f32,
}

impl Vec3 {
    fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    #[cfg(test)]
    fn add(self, o: Self) -> Self {
        Self::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }

    fn sub(self, o: Self) -> Self {
        Self::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }

    fn scale(self, k: f32) -> Self {
        Self::new(self.x * k, self.y * k, self.z * k)
    }

    fn dot(self, o: Self) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    fn cross(self, o: Self) -> Self {
        Self::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }

    fn len_sq(self) -> f32 {
        self.x * self.x + self.y * self.y + self.z * self.z
    }
}

struct MeshProjector {
    center: Pos2,
    scale_xy: f32,
    scale_z: f32,
    yaw: f32,
    pitch: f32,
    /// Unit vector from the surface toward the camera, in model space.
    view_dir: Vec3,
    light_dir: Vec3,
    fit_scale: f32,
    fit_offset: Vec2,
}

impl MeshProjector {
    fn new(
        rect: Rect,
        rows: usize,
        cols: usize,
        yaw: f32,
        pitch: f32,
        zoom: f32,
        z_soften: bool,
    ) -> Self {
        let inner = rect.shrink(8.0);
        let grid_span = (rows.max(cols) as f32).max(1.0);
        let zoom = zoom.clamp(0.4, 2.4);
        let fit = inner.width().min(inner.height()) * 0.78;
        let scale_xy = fit / grid_span;
        // Z amplitude ~55% of table footprint (Haltech/MoTeC style). Do not tie scale_z to a
        // single cell — that crushed relief after bbox auto-fit (PR #9).
        let scale_z = fit * 0.55;

        let (sin_p, cos_p) = pitch.sin_cos();
        let (sin_y, cos_y) = yaw.sin_cos();
        // Inverse of `rotate` applied to the rotated-space camera direction (0, -1, 0).
        let view_dir = Vec3::new(-sin_y * cos_p, -cos_y * cos_p, sin_p);
        let l = Vec3::new(0.4, -0.45, 0.78);
        let light_dir = l.scale(1.0 / l.len_sq().sqrt());

        let mut proj = Self {
            center: inner.center(),
            scale_xy,
            scale_z,
            yaw,
            pitch,
            view_dir,
            light_dir,
            fit_scale: 1.0,
            fit_offset: Vec2::ZERO,
        };
        proj.fit_to_rect(inner, rows, cols, z_soften, zoom);
        proj
    }

    /// Fit the projected bounding box into `inner`, then apply user zoom around the centre.
    fn fit_to_rect(&mut self, inner: Rect, rows: usize, cols: usize, z_soften: bool, zoom: f32) {
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;

        let z_samples = [display_z_norm(0.0, z_soften), display_z_norm(1.0, z_soften)];
        let corners = [
            (0.0, 0.0),
            ((cols - 1) as f32, 0.0),
            (0.0, (rows - 1) as f32),
            ((cols - 1) as f32, (rows - 1) as f32),
        ];
        for &(c, r) in &corners {
            for &zn in &z_samples {
                let p = self.model_point(c, r, zn, cols, rows);
                let (s, _) = self.project_raw(p);
                min_x = min_x.min(s.x);
                min_y = min_y.min(s.y);
                max_x = max_x.max(s.x);
                max_y = max_y.max(s.y);
            }
        }

        let bw = (max_x - min_x).max(1.0);
        let bh = (max_y - min_y).max(1.0);
        let margin = 0.82;
        let fit = (inner.width() * margin / bw).min(inner.height() * margin / bh) * zoom;
        let bbox_cx = (min_x + max_x) * 0.5;
        let bbox_cy = (min_y + max_y) * 0.5;
        self.fit_scale = fit;
        self.fit_offset = Vec2::new(
            inner.center().x - bbox_cx * fit,
            inner.center().y - bbox_cy * fit,
        );
    }

    fn model_point(&self, col: f32, row: f32, z_norm: f32, cols: usize, rows: usize) -> Vec3 {
        let x = (col - (cols - 1) as f32 * 0.5) * self.scale_xy;
        let y = (row - (rows - 1) as f32 * 0.5) * self.scale_xy;
        let z = (z_norm - 0.5) * self.scale_z;
        Vec3::new(x, y, z)
    }

    fn rotate(&self, p: Vec3) -> Vec3 {
        let (sin_y, cos_y) = self.yaw.sin_cos();
        let (sin_p, cos_p) = self.pitch.sin_cos();

        let x1 = p.x * cos_y - p.y * sin_y;
        let y1 = p.x * sin_y + p.y * cos_y;
        let z1 = p.z;

        let y2 = y1 * cos_p - z1 * sin_p;
        let z2 = y1 * sin_p + z1 * cos_p;
        Vec3::new(x1, y2, z2)
    }

    /// Screen position (before fit) and depth; larger depth = farther from the camera.
    fn project_raw(&self, p: Vec3) -> (Pos2, f32) {
        let r = self.rotate(p);
        let screen = Pos2::new(self.center.x + r.x, self.center.y - r.z);
        (screen, r.y)
    }

    fn project(&self, p: Vec3) -> (Pos2, f32) {
        let (raw, depth) = self.project_raw(p);
        let screen = Pos2::new(
            raw.x * self.fit_scale + self.fit_offset.x,
            raw.y * self.fit_scale + self.fit_offset.y,
        );
        (screen, depth)
    }

    /// Upward-oriented unit normal (a heightfield's "top" side is +Z).
    fn face_normal(&self, a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
        let n = b.sub(a).cross(c.sub(a));
        if n.len_sq() < 1e-8 {
            return Vec3::new(0.0, 0.0, 1.0);
        }
        let n = n.scale(1.0 / n.len_sq().sqrt());
        if n.z < 0.0 {
            n.scale(-1.0)
        } else {
            n
        }
    }

    /// The surface is open (not a closed solid), so faces seen from underneath are drawn
    /// darker rather than culled — culling left holes at steep cliffs.
    fn shade(&self, normal: Vec3, base: Color32) -> Color32 {
        let facing = normal.dot(self.view_dir);
        if facing < 0.0 {
            return base.gamma_multiply(0.38);
        }
        let ndotl = normal.dot(self.light_dir).max(0.0);
        let factor = (0.5 + 0.45 * ndotl + 0.08 * facing).clamp(0.45, 1.15);
        base.gamma_multiply(factor)
    }
}

/// Display-only Z compression for sharp MAP shelves / cliffs (Spark, etc.). Linear for normal VE maps.
fn display_z_norm(raw: f32, soften: bool) -> f32 {
    if !soften {
        return raw;
    }
    let d = raw - 0.5;
    0.5 + d.signum() * d.abs().powf(0.65) * 0.72
}

/// True when any neighbor step exceeds ~35% of the table value span (outlier shelf/cliff).
fn table_needs_z_softening(values: &[Vec<f64>]) -> bool {
    let rows = values.len();
    let cols = values.first().map(|r| r.len()).unwrap_or(0);
    if rows < 2 || cols < 2 {
        return false;
    }

    let (min_v, max_v) = value_range(values);
    let span = max_v - min_v;
    if span < 1e-9 {
        return false;
    }
    let threshold = span * 0.35;

    for r in 0..rows {
        for c in 0..cols {
            let v = values[r][c];
            if c + 1 < cols && (values[r][c + 1] - v).abs() > threshold {
                return true;
            }
            if r + 1 < rows && (values[r + 1][c] - v).abs() > threshold {
                return true;
            }
        }
    }
    false
}

fn value_range(values: &[Vec<f64>]) -> (f64, f64) {
    let mut min_v = f64::MAX;
    let mut max_v = f64::MIN;
    for row in values {
        for &v in row {
            min_v = min_v.min(v);
            max_v = max_v.max(v);
        }
    }
    (min_v, max_v)
}

/// Compact number for axis ticks and the hover readout.
fn fmt_val(v: f64) -> String {
    let a = v.abs();
    if a >= 1000.0 || (a >= 100.0 && v.fract().abs() < 1e-6) {
        format!("{:.0}", v)
    } else if a >= 10.0 {
        format!("{:.1}", v)
    } else {
        format!("{:.2}", v)
    }
}

fn axis_name(units: &str, fallback: &str) -> String {
    let u = units.trim();
    if u.is_empty() {
        fallback.to_string()
    } else {
        u.to_string()
    }
}

/// Shared geometry for painting and picking (identical projection = picks match pixels).
struct MeshScene<'a> {
    table: &'a XdfTable,
    rows: usize,
    cols: usize,
    min_val: f64,
    max_val: f64,
    z_soften: bool,
    proj: MeshProjector,
}

impl<'a> MeshScene<'a> {
    fn new(rect: Rect, table: &'a XdfTable, yaw: f32, pitch: f32, zoom: f32) -> Option<Self> {
        let rows = table.values.len();
        let cols = table.values.first().map(|r| r.len()).unwrap_or(0);
        if rows < 2 || cols < 2 || table.values.iter().any(|r| r.len() != cols) {
            return None;
        }
        let (min_val, max_val) = value_range(&table.values);
        let z_soften = table_needs_z_softening(&table.values);
        let proj = MeshProjector::new(rect, rows, cols, yaw, pitch, zoom, z_soften);
        Some(Self {
            table,
            rows,
            cols,
            min_val,
            max_val,
            z_soften,
            proj,
        })
    }

    fn z_norm(&self, r: usize, c: usize) -> f32 {
        let val = self.table.values[r][c];
        let raw = if self.max_val > self.min_val {
            ((val - self.min_val) / (self.max_val - self.min_val)) as f32
        } else {
            0.5
        };
        display_z_norm(raw, self.z_soften)
    }

    /// Surface point at a fractional `(row, col)`, bilinear like the ECU's own table lookup.
    fn surface_point(&self, row: f32, col: f32) -> Vec3 {
        let row = row.clamp(0.0, (self.rows - 1) as f32);
        let col = col.clamp(0.0, (self.cols - 1) as f32);
        let r0 = (row.floor() as usize).min(self.rows - 2);
        let c0 = (col.floor() as usize).min(self.cols - 2);
        let (tr, tc) = (row - r0 as f32, col - c0 as f32);
        let z = |r, c| self.z_norm(r, c);
        let top = z(r0, c0) * (1.0 - tc) + z(r0, c0 + 1) * tc;
        let bot = z(r0 + 1, c0) * (1.0 - tc) + z(r0 + 1, c0 + 1) * tc;
        let zn = top * (1.0 - tr) + bot * tr;
        self.proj.model_point(col, row, zn, self.cols, self.rows)
    }

    fn floor_z(&self) -> f32 {
        display_z_norm(0.0, self.z_soften)
    }

    fn vertex(&self, r: usize, c: usize) -> Vec3 {
        self.proj
            .model_point(c as f32, r as f32, self.z_norm(r, c), self.cols, self.rows)
    }

    fn floor_point(&self, col: f32, row: f32) -> Vec3 {
        self.proj
            .model_point(col, row, self.floor_z(), self.cols, self.rows)
    }

    /// Nearest visible-ish vertex to `pos`; among close candidates the one nearest the camera wins.
    fn pick(&self, pos: Pos2) -> Option<(usize, usize)> {
        let mut best: Option<((usize, usize), f32, f32)> = None;
        for r in 0..self.rows {
            for c in 0..self.cols {
                let (s, depth) = self.proj.project(self.vertex(r, c));
                let dist = s.distance(pos);
                if dist > PICK_RADIUS {
                    continue;
                }
                let better = match best {
                    None => true,
                    // Within a few px, prefer the front-most vertex; otherwise the closest.
                    Some((_, bd, bdepth)) => {
                        if (dist - bd).abs() < 4.0 {
                            depth < bdepth
                        } else {
                            dist < bd
                        }
                    }
                };
                if better {
                    best = Some(((r, c), dist, depth));
                }
            }
        }
        best.map(|(cell, _, _)| cell)
    }
}

/// Cell `(row, col)` under `pos`, using the same projection as [`paint_surface_mesh`].
pub fn pick_cell(
    rect: Rect,
    table: &XdfTable,
    yaw: f32,
    pitch: f32,
    zoom: f32,
    pos: Pos2,
) -> Option<(usize, usize)> {
    MeshScene::new(rect, table, yaw, pitch, zoom)?.pick(pos)
}

struct CellCmd {
    depth: f32,
    tris: [[Pos2; 3]; 2],
    fills: [Color32; 2],
    edges: [Pos2; 4],
}

/// Paint the 3D surface mesh into `rect` using the full available canvas size.
pub fn paint_surface_mesh(
    painter: &Painter,
    rect: Rect,
    table: &XdfTable,
    yaw: f32,
    pitch: f32,
    zoom: f32,
    active_cell: (usize, usize),
    hover_cell: Option<(usize, usize)>,
    live_trail: &[((f32, f32), f32)],
    live_off_table: bool,
) {
    painter.rect_filled(rect, 4.0, BG_FILL);
    painter.rect_stroke(rect, 4.0, Stroke::new(1.0, STROKE));

    let scene = match MeshScene::new(rect, table, yaw, pitch, zoom) {
        Some(s) => s,
        None => {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "No mesh data (needs a 2D table)",
                FontId::proportional(12.0),
                MUTED,
            );
            return;
        }
    };
    let clip = painter.with_clip_rect(rect.expand(-1.0));
    let proj = &scene.proj;
    let (rows, cols) = (scene.rows, scene.cols);

    // 1. Floor grid + axes first so the surface correctly hides them.
    paint_floor_grid(&scene, &clip);
    let axis_labels = paint_axes(&scene, &clip);

    // 2. Surface: one command per cell, painted back to front; each cell draws its own
    //    grid edges right after its fill, so nearer cells hide farther wires.
    let mut cmds: Vec<CellCmd> = Vec::with_capacity((rows - 1) * (cols - 1));
    for r in 0..rows - 1 {
        for c in 0..cols - 1 {
            let p00 = scene.vertex(r, c);
            let p01 = scene.vertex(r, c + 1);
            let p10 = scene.vertex(r + 1, c);
            let p11 = scene.vertex(r + 1, c + 1);
            let (s00, d00) = proj.project(p00);
            let (s01, d01) = proj.project(p01);
            let (s10, d10) = proj.project(p10);
            let (s11, d11) = proj.project(p11);

            let v = &table.values;
            let tri_fill = |a: Vec3, b: Vec3, cc: Vec3, mean: f64| {
                let base = heatmap_color(mean, scene.min_val, scene.max_val).gamma_multiply(0.92);
                proj.shade(proj.face_normal(a, b, cc), base)
            };
            let f0 = tri_fill(p00, p10, p11, (v[r][c] + v[r + 1][c] + v[r + 1][c + 1]) / 3.0);
            let f1 = tri_fill(p00, p11, p01, (v[r][c] + v[r + 1][c + 1] + v[r][c + 1]) / 3.0);

            cmds.push(CellCmd {
                depth: (d00 + d01 + d10 + d11) * 0.25,
                tris: [[s00, s10, s11], [s00, s11, s01]],
                fills: [f0, f1],
                edges: [s00, s01, s11, s10],
            });
        }
    }
    cmds.sort_by(|a, b| b.depth.partial_cmp(&a.depth).unwrap_or(std::cmp::Ordering::Equal));

    let edge = Stroke::new(0.8, Color32::from_rgba_unmultiplied(8, 12, 18, 150));
    for cmd in cmds {
        // Fill-only + separate segments: stroked paths get miter spikes when a cell is seen
        // nearly edge-on (degenerate triangles), which shot lines across the viewport.
        for (tri, fill) in cmd.tris.iter().zip(cmd.fills) {
            if is_screen_sliver(tri) {
                continue;
            }
            clip.add(Shape::convex_polygon(tri.to_vec(), fill, Stroke::NONE));
        }
        let e = cmd.edges;
        for i in 0..4 {
            clip.line_segment([e[i], e[(i + 1) % 4]], edge);
        }
    }

    // 3. Overlays. The trail goes first so axis labels and the cursor stay readable on top.
    paint_live_trail(&scene, &clip, live_trail, live_off_table);
    let mut placed = Vec::new();
    for (anchor, label, color) in &axis_labels {
        paint_axis_label_avoiding(&clip, *anchor, label, *color, &mut placed);
    }
    if let Some(cell) = hover_cell {
        if cell != active_cell {
            paint_cell_marker(&scene, &clip, cell, TEXT.gamma_multiply(0.85), false);
        }
    }
    paint_cell_marker(&scene, &clip, active_cell, ACCENT_CYAN, true);
    paint_readout(&scene, &clip, rect, hover_cell.unwrap_or(active_cell), hover_cell.is_some());

    let mut badge = format!("{:.0}%", zoom * 100.0);
    if scene.z_soften {
        badge = format!("Z compressed (display only) - {}", badge);
    }
    clip.text(
        rect.right_top() + Vec2::new(-6.0, 4.0),
        Align2::RIGHT_TOP,
        badge,
        FontId::monospace(9.0),
        MUTED,
    );
}

/// Edge-on triangle (< 0.5 px tall on screen). egui's anti-alias fringe extrudes such
/// slivers into long spikes across the viewport; they cover no pixels, so skip them.
fn is_screen_sliver(tri: &[Pos2; 3]) -> bool {
    let [a, b, c] = *tri;
    let twice_area = ((b - a).x * (c - a).y - (b - a).y * (c - a).x).abs();
    let longest = a.distance(b).max(b.distance(c)).max(c.distance(a));
    longest < 1e-3 || twice_area / longest < 0.5
}

fn paint_floor_grid(scene: &MeshScene, painter: &Painter) {
    let proj = &scene.proj;
    let stroke = Stroke::new(0.6, STROKE.gamma_multiply(0.55));
    let (rows, cols) = (scene.rows, scene.cols);
    for r in 0..rows {
        let (a, _) = proj.project(scene.floor_point(0.0, r as f32));
        let (b, _) = proj.project(scene.floor_point((cols - 1) as f32, r as f32));
        painter.line_segment([a, b], stroke);
    }
    for c in 0..cols {
        let (a, _) = proj.project(scene.floor_point(c as f32, 0.0));
        let (b, _) = proj.project(scene.floor_point(c as f32, (rows - 1) as f32));
        painter.line_segment([a, b], stroke);
    }
}

/// Boxed label, shifted so it stays fully inside the mesh viewport.
fn paint_axis_label(painter: &Painter, anchor: Pos2, label: &str, color: Color32) {
    paint_axis_label_avoiding(painter, anchor, label, color, &mut Vec::new());
}

/// Like [`paint_axis_label`], but nudged vertically off any rect in `placed` (then added to it),
/// so axis labels that project to the same corner don't stack on top of each other.
fn paint_axis_label_avoiding(
    painter: &Painter,
    anchor: Pos2,
    label: &str,
    color: Color32,
    placed: &mut Vec<Rect>,
) {
    let font = FontId::proportional(10.0);
    let galley = painter.layout_no_wrap(label.into(), font, color);
    let bounds = painter.clip_rect().shrink(4.0);
    let size = galley.size();
    let clamp = |p: Pos2| {
        Pos2::new(
            p.x.min(bounds.right() - size.x).max(bounds.left()),
            p.y.min(bounds.bottom() - size.y).max(bounds.top()),
        )
    };
    let base = clamp(anchor);
    let mut anchor = base;
    let step = size.y + 8.0;
    for attempt in 0..6 {
        let r = Rect::from_min_size(anchor, size).expand(3.0);
        if !placed.iter().any(|p| p.intersects(r)) {
            break;
        }
        // Alternate below / above the original spot: +1, -1, +2, -2, ...
        let k = (attempt / 2 + 1) as f32 * if attempt % 2 == 0 { 1.0 } else { -1.0 };
        anchor = clamp(base + Vec2::new(0.0, step * k));
    }
    let rect = Rect::from_min_size(anchor, size).expand(3.0);
    placed.push(rect);
    painter.rect_filled(rect, 2.0, BG_FILL.gamma_multiply(0.92));
    painter.rect_stroke(rect, 2.0, Stroke::new(1.0, STROKE.gamma_multiply(0.8)));
    painter.galley(anchor, galley, color);
}

fn axis_tick(labels: &[f64], idx: usize) -> Option<String> {
    labels.get(idx).map(|v| fmt_val(*v))
}

/// Axes run along the full table edges from the (row 0, col 0) floor corner, labelled with the
/// table's own units and end values (not hard-coded RPM/MAP). Lines are painted now (under the
/// surface); the returned labels are painted after the surface so it never hides them.
fn paint_axes(scene: &MeshScene, painter: &Painter) -> Vec<(Pos2, String, Color32)> {
    let proj = &scene.proj;
    let t = scene.table;
    let (rows, cols) = (scene.rows, scene.cols);
    let o = scene.floor_point(0.0, 0.0);
    let x_end = scene.floor_point((cols - 1) as f32, 0.0);
    let y_end = scene.floor_point(0.0, (rows - 1) as f32);
    let z_end = proj.model_point(0.0, 0.0, display_z_norm(1.0, scene.z_soften), cols, rows);

    let x_col = ACCENT_CYAN;
    let y_col = Color32::from_rgb(0xF5, 0x9E, 0x0B);
    let z_col = Color32::from_rgb(0x2F, 0x81, 0xF7);

    let mut labels = Vec::new();
    let mut draw_axis = |from: Vec3, to: Vec3, color: Color32, label: String| {
        let (a, _) = proj.project(from);
        let (b, _) = proj.project(to);
        painter.line_segment([a, b], Stroke::new(1.75, color));
        labels.push((b + Vec2::new(3.0, -3.0), label, color));
    };

    let range = |lo: Option<String>, hi: Option<String>| match (lo, hi) {
        (Some(l), Some(h)) => format!(" {}-{}", l, h),
        _ => String::new(),
    };
    let x_name = axis_name(&t.x_axis.units, "X");
    let y_name = axis_name(&t.y_axis.units, "Y");
    let z_name = axis_name(&t.z_units, "Value");
    draw_axis(
        o,
        x_end,
        x_col,
        format!("{}{}", x_name, range(axis_tick(&t.x_axis.labels, 0), axis_tick(&t.x_axis.labels, cols - 1))),
    );
    draw_axis(
        o,
        y_end,
        y_col,
        format!("{}{}", y_name, range(axis_tick(&t.y_axis.labels, 0), axis_tick(&t.y_axis.labels, rows - 1))),
    );
    draw_axis(
        o,
        z_end,
        z_col,
        format!("{} {}-{}", z_name, fmt_val(scene.min_val), fmt_val(scene.max_val)),
    );

    let (o_screen, _) = proj.project(o);
    painter.circle_filled(o_screen, 2.5, TEXT);
    labels
}

fn paint_cell_marker(
    scene: &MeshScene,
    painter: &Painter,
    cell: (usize, usize),
    color: Color32,
    primary: bool,
) {
    let (r, c) = cell;
    if r >= scene.rows || c >= scene.cols {
        return;
    }
    let (pos, _) = scene.proj.project(scene.vertex(r, c));
    let (base, _) = scene.proj.project(scene.floor_point(c as f32, r as f32));
    painter.line_segment([base, pos], Stroke::new(1.25, color.gamma_multiply(0.45)));
    if primary {
        painter.circle_filled(pos, 4.5, color);
        painter.circle_stroke(pos, 7.0, Stroke::new(2.0, TEXT));
        painter.circle_stroke(pos, 11.0, Stroke::new(1.0, color.gamma_multiply(0.35)));
    } else {
        painter.circle_stroke(pos, 5.5, Stroke::new(1.5, color));
    }
}

/// Live operating point: fading green trail riding the surface, bright head = now.
/// Drawn over the mesh (an overlay like the cursor), lifted slightly so it never z-fights.
fn paint_live_trail(scene: &MeshScene, painter: &Painter, trail: &[((f32, f32), f32)], off_table: bool) {
    if trail.is_empty() {
        return;
    }
    let lift = Vec3::new(0.0, 0.0, scene.proj.scale_z * 0.015);
    let screen: Vec<(Pos2, f32)> = trail
        .iter()
        .map(|&((r, c), age)| {
            let p = scene.surface_point(r, c);
            let (s, _) = scene.proj.project(Vec3::new(p.x + lift.x, p.y + lift.y, p.z + lift.z));
            (s, age)
        })
        .collect();
    for w in screen.windows(2) {
        let fade = 1.0 - w[1].1;
        painter.line_segment(
            [w[0].0, w[1].0],
            Stroke::new(1.0 + 2.0 * fade, EMERALD.gamma_multiply(0.15 + 0.85 * fade)),
        );
    }
    let (head, _) = *screen.last().unwrap();
    let (r, c) = trail.last().unwrap().0;
    let (base, _) = scene.proj.project(scene.floor_point(c, r));
    painter.line_segment([base, head], Stroke::new(1.0, EMERALD.gamma_multiply(0.4)));
    // Amber head + warning ring when the engine is past the table edge (value clamped).
    let head_col = if off_table { Color32::from_rgb(255, 152, 0) } else { EMERALD };
    painter.circle_filled(head, 5.0, head_col);
    painter.circle_stroke(head, 8.5, Stroke::new(1.5, head_col.gamma_multiply(0.5)));
    if off_table {
        painter.circle_stroke(head, 13.0, Stroke::new(1.5, head_col));
    }
}

/// Bottom-left readout: `X × Y → value` for the hovered (or selected) cell.
fn paint_readout(scene: &MeshScene, painter: &Painter, rect: Rect, cell: (usize, usize), hovering: bool) {
    let (r, c) = cell;
    if r >= scene.rows || c >= scene.cols {
        return;
    }
    let t = scene.table;
    let x = axis_tick(&t.x_axis.labels, c).unwrap_or_else(|| format!("col {}", c));
    let y = axis_tick(&t.y_axis.labels, r).unwrap_or_else(|| format!("row {}", r));
    let z_units = t.z_units.trim();
    // ASCII-only separators: the bundled font has no glyph for arrows / bullets.
    let text = format!(
        "{}{} {} x {} {} = {}{}{}",
        if hovering { "" } else { "Selected: " },
        x,
        axis_name(&t.x_axis.units, ""),
        y,
        axis_name(&t.y_axis.units, ""),
        fmt_val(t.values[r][c]),
        if z_units.is_empty() { "" } else { " " },
        z_units
    );
    let anchor = rect.left_bottom() + Vec2::new(8.0, -20.0);
    paint_axis_label(painter, anchor, &text, if hovering { TEXT } else { ACCENT_CYAN });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proj_at(yaw: f32, pitch: f32, n: usize) -> MeshProjector {
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(200.0, 200.0));
        MeshProjector::new(rect, n, n, yaw, pitch, 1.0, false)
    }

    #[test]
    fn z_amplitude_matches_table_footprint() {
        let proj = proj_at(0.8, 0.5, 16);
        let ratio = proj.scale_z / (16.0 * proj.scale_xy);
        assert!(
            ratio > 0.45 && ratio < 0.65,
            "Z height should be ~55% of table span, got ratio {}",
            ratio
        );
    }

    #[test]
    fn projector_depth_orders_near_before_far() {
        let proj = proj_at(0.8, 0.5, 4);
        let near = proj.model_point(2.0, 2.0, display_z_norm(0.9, false), 4, 4);
        let far = proj.model_point(2.0, 2.0, display_z_norm(0.1, false), 4, 4);
        let (_, d_near) = proj.project(near);
        let (_, d_far) = proj.project(far);
        assert!(d_near < d_far, "higher Z norm should be closer to camera");
    }

    /// Regression: depth used to mix rotated coords with a model-space view vector, which
    /// inverted row ordering above ~45° pitch.
    #[test]
    fn front_row_is_nearer_at_steep_pitch_and_any_yaw() {
        for &pitch in &[0.3f32, 0.8, 1.2, 1.4] {
            for &yaw in &[0.0f32, 0.785, 2.0, -1.0] {
                let proj = proj_at(yaw, pitch, 8);
                // Move one step toward the camera along its horizontal look direction.
                let base = proj.model_point(3.5, 3.5, 0.5, 8, 8);
                let toward = Vec3::new(proj.view_dir.x, proj.view_dir.y, 0.0);
                let (_, d_base) = proj.project(base);
                let (_, d_near) = proj.project(base.add(toward.scale(10.0)));
                assert!(d_near < d_base, "yaw {} pitch {}", yaw, pitch);
            }
        }
    }

    #[test]
    fn view_dir_points_at_camera() {
        // Rotating the model-space view vector must give the rotated-space camera direction.
        for &yaw in &[0.0f32, 0.785, 2.5] {
            let proj = proj_at(yaw, 0.6, 4);
            let r = proj.rotate(proj.view_dir);
            assert!(r.x.abs() < 1e-4 && (r.y + 1.0).abs() < 1e-4 && r.z.abs() < 1e-4, "yaw {}", yaw);
        }
    }

    #[test]
    fn face_normal_points_up_regardless_of_winding() {
        let proj = proj_at(0.0, 1.2, 3);
        let p0 = proj.model_point(0.0, 0.0, 0.5, 3, 3);
        let p1 = proj.model_point(1.0, 0.0, 0.5, 3, 3);
        let p2 = proj.model_point(0.0, 1.0, 0.5, 3, 3);
        assert!(proj.face_normal(p0, p1, p2).z > 0.99);
        assert!(proj.face_normal(p0, p2, p1).z > 0.99);
    }

    #[test]
    fn display_z_norm_linear_when_not_softening() {
        assert_eq!(display_z_norm(0.25, false), 0.25);
        assert_eq!(display_z_norm(0.75, false), 0.75);
    }

    #[test]
    fn display_z_norm_softens_extremes_when_enabled() {
        assert!(display_z_norm(1.0, true) < 1.0);
        assert!(display_z_norm(0.0, true) > 0.0);
    }

    #[test]
    fn gradual_ve_table_does_not_soften() {
        let rows = 8usize;
        let cols = 8usize;
        let mut values = vec![vec![0.0; cols]; rows];
        for r in 0..rows {
            for c in 0..cols {
                values[r][c] = 55.0 + (r + c) as f64 * 2.5;
            }
        }
        assert!(!table_needs_z_softening(&values));
    }

    #[test]
    fn sharp_shelf_triggers_softening() {
        let values = vec![vec![50.0, 50.0, 95.0, 95.0], vec![50.0, 50.0, 95.0, 95.0]];
        assert!(table_needs_z_softening(&values));
    }

    #[test]
    fn sliver_detection() {
        let p = |x: f32, y: f32| Pos2::new(x, y);
        assert!(is_screen_sliver(&[p(0.0, 0.0), p(100.0, 0.1), p(200.0, 0.0)]));
        assert!(!is_screen_sliver(&[p(0.0, 0.0), p(10.0, 0.0), p(0.0, 10.0)]));
    }

    #[test]
    fn fmt_val_is_compact() {
        assert_eq!(fmt_val(6000.0), "6000");
        assert_eq!(fmt_val(14.7), "14.7");
        assert_eq!(fmt_val(0.85), "0.85");
    }
}
