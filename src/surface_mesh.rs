//! Isometric 3D calibration surface mesh with painter's-algorithm depth sorting.

use crate::definitions::XdfTable;
use crate::theme::{heatmap_color, ACCENT_CYAN, BG_FILL, MUTED, STROKE, TEXT};
use eframe::egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Shape, Stroke, Vec2};

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

    fn add(self, o: Self) -> Self {
        Self::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }

    fn sub(self, o: Self) -> Self {
        Self::new(self.x - o.x, self.y - o.y, self.z - o.z)
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
    view_dir: Vec3,
    light_dir: Vec3,
    fit_scale: f32,
    fit_offset: Vec2,
}

impl MeshProjector {
    fn new(rect: Rect, rows: usize, cols: usize, yaw: f32, pitch: f32, zoom: f32) -> Self {
        let inner = rect.shrink(8.0);
        let grid_span = (rows.max(cols) as f32).max(1.0);
        let fit_base = inner.width().min(inner.height()) * 0.78 / grid_span;
        let scale_xy = fit_base * zoom.clamp(0.4, 2.4);
        let scale_z = scale_xy * 0.48;

        let cos_p = pitch.cos();
        let sin_p = pitch.sin();
        let cos_y = yaw.cos();
        let sin_y = yaw.sin();

        let view_dir = Vec3::new(sin_y * cos_p, -cos_y * cos_p, sin_p);
        let light_dir = Vec3::new(0.4, -0.45, 0.78);

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
        proj.fit_to_rect(inner, rows, cols);
        proj
    }

    fn fit_to_rect(&mut self, inner: Rect, rows: usize, cols: usize) {
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;

        for r in 0..rows {
            for c in 0..cols {
                for &zn in &[0.0_f32, 0.5, 1.0] {
                    let p = self.model_point(c as f32, r as f32, zn, cols, rows);
                    let (s, _) = self.project_raw(p);
                    min_x = min_x.min(s.x);
                    min_y = min_y.min(s.y);
                    max_x = max_x.max(s.x);
                    max_y = max_y.max(s.y);
                }
            }
        }

        let bw = (max_x - min_x).max(1.0);
        let bh = (max_y - min_y).max(1.0);
        let margin = 0.9;
        let fit = (inner.width() * margin / bw).min(inner.height() * margin / bh);
        let bbox_cx = (min_x + max_x) * 0.5;
        let bbox_cy = (min_y + max_y) * 0.5;
        self.fit_scale = fit;
        self.fit_offset = Vec2::new(
            inner.center().x - bbox_cx * fit,
            inner.center().y - bbox_cy * fit,
        );
    }

    fn model_point(&self, col: f32, row: f32, z_norm: f32, cols: usize, rows: usize) -> Vec3 {
        let x = (col - cols as f32 * 0.5) * self.scale_xy;
        let y = (row - rows as f32 * 0.5) * self.scale_xy;
        let z = (z_norm - 0.5) * self.scale_z;
        Vec3::new(x, y, z)
    }

    fn rotate(&self, p: Vec3) -> Vec3 {
        let cos_y = self.yaw.cos();
        let sin_y = self.yaw.sin();
        let cos_p = self.pitch.cos();
        let sin_p = self.pitch.sin();

        let x1 = p.x * cos_y - p.y * sin_y;
        let y1 = p.x * sin_y + p.y * cos_y;
        let z1 = p.z;

        let y2 = y1 * cos_p - z1 * sin_p;
        let z2 = y1 * sin_p + z1 * cos_p;
        Vec3::new(x1, y2, z2)
    }

    fn project_raw(&self, p: Vec3) -> (Pos2, f32) {
        let r = self.rotate(p);
        let depth = -(r.x * self.view_dir.x + r.y * self.view_dir.y + r.z * self.view_dir.z);
        let screen = Pos2::new(self.center.x + r.x, self.center.y - r.z);
        (screen, depth)
    }

    fn project(&self, p: Vec3) -> (Pos2, f32) {
        let (raw, depth) = self.project_raw(p);
        let screen = Pos2::new(
            raw.x * self.fit_scale + self.fit_offset.x,
            raw.y * self.fit_scale + self.fit_offset.y,
        );
        (screen, depth)
    }

    fn face_normal(&self, a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
        let ab = b.sub(a);
        let ac = c.sub(a);
        let n = ab.cross(ac);
        if n.len_sq() < 1e-8 {
            return Vec3::new(0.0, 0.0, 1.0);
        }
        let inv = 1.0 / n.len_sq().sqrt();
        Vec3::new(n.x * inv, n.y * inv, n.z * inv)
    }

    fn shade(&self, normal: Vec3, base: Color32, facing: f32) -> Color32 {
        let ndotl = normal.dot(self.light_dir).max(0.0);
        let ambient = 0.52;
        let diffuse = 0.48 * ndotl;
        let rim = 0.08 * facing.max(0.0);
        let factor = (ambient + diffuse + rim).clamp(0.5, 1.2);
        base.gamma_multiply(factor)
    }
}

struct FillTriangleCmd {
    depth: f32,
    pts: [Pos2; 3],
    fill: Color32,
    stroke: Stroke,
}

/// Display-only Z compression so sharp table cliffs read as slopes, not trenches.
fn display_z_norm(raw: f32) -> f32 {
    let d = raw - 0.5;
    0.5 + d.signum() * d.abs().sqrt() * 0.62
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
) {
    painter.rect_filled(rect, 4.0, BG_FILL);
    painter.rect_stroke(rect, 4.0, Stroke::new(1.0, STROKE));

    let rows = table.values.len();
    let cols = table.values.first().map(|r| r.len()).unwrap_or(0);
    if rows < 2 || cols < 2 {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "No mesh data",
            FontId::proportional(12.0),
            MUTED,
        );
        return;
    }

    let mut min_val = f64::MAX;
    let mut max_val = f64::MIN;
    for r in 0..rows {
        for c in 0..cols {
            let v = table.values[r][c];
            min_val = min_val.min(v);
            max_val = max_val.max(v);
        }
    }

    let proj = MeshProjector::new(rect, rows, cols, yaw, pitch, zoom);
    let clip = painter.with_clip_rect(rect.expand(-1.0));
    let mut cmds: Vec<FillTriangleCmd> = Vec::new();

    let z_norm = |r: usize, c: usize| -> f32 {
        let val = table.values[r][c];
        let raw = if max_val > min_val {
            ((val - min_val) / (max_val - min_val)) as f32
        } else {
            0.5
        };
        display_z_norm(raw)
    };

    let mut push_tri = |r0: usize, c0: usize, r1: usize, c1: usize, r2: usize, c2: usize| {
        let p0 = proj.model_point(c0 as f32, r0 as f32, z_norm(r0, c0), cols, rows);
        let p1 = proj.model_point(c1 as f32, r1 as f32, z_norm(r1, c1), cols, rows);
        let p2 = proj.model_point(c2 as f32, r2 as f32, z_norm(r2, c2), cols, rows);

        let normal = proj.face_normal(p0, p1, p2);
        let facing = normal.dot(proj.view_dir);
        if facing <= 0.0 {
            return;
        }

        let (s0, d0) = proj.project(p0);
        let (s1, d1) = proj.project(p1);
        let (s2, d2) = proj.project(p2);
        let depth = (d0 + d1 + d2) / 3.0;
        let mid_val = (table.values[r0][c0] + table.values[r1][c1] + table.values[r2][c2]) / 3.0;
        let base = heatmap_color(mid_val, min_val, max_val);
        let fill = proj.shade(normal, base.gamma_multiply(0.9), facing);
        cmds.push(FillTriangleCmd {
            depth,
            pts: [s0, s1, s2],
            fill,
            stroke: Stroke::new(0.55, Color32::from_rgba_unmultiplied(6, 10, 16, 140)),
        });
    };

    for r in 0..rows - 1 {
        for c in 0..cols - 1 {
            push_tri(r, c, r + 1, c, r + 1, c + 1);
            push_tri(r, c, r + 1, c + 1, r, c + 1);
        }
    }

    cmds.sort_by(|a, b| {
        b.depth
            .partial_cmp(&a.depth)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    for cmd in cmds {
        clip.add(Shape::convex_polygon(cmd.pts.to_vec(), cmd.fill, cmd.stroke));
    }

    let wire_stroke = Stroke::new(0.85, Color32::from_rgba_unmultiplied(48, 56, 68, 200));
    let row_stroke = Stroke::new(1.05, Color32::from_rgba_unmultiplied(90, 100, 118, 220));
    for r in 0..rows {
        let stroke = if r == rows - 1 {
            row_stroke
        } else {
            wire_stroke
        };
        for c in 0..cols - 1 {
            let pa = proj.model_point(c as f32, r as f32, z_norm(r, c), cols, rows);
            let pb = proj.model_point((c + 1) as f32, r as f32, z_norm(r, c + 1), cols, rows);
            let (sa, _) = proj.project(pa);
            let (sb, _) = proj.project(pb);
            clip.line_segment([sa, sb], stroke);
        }
    }
    for c in 0..cols {
        for r in 0..rows - 1 {
            let pa = proj.model_point(c as f32, r as f32, z_norm(r, c), cols, rows);
            let pb = proj.model_point(c as f32, (r + 1) as f32, z_norm(r + 1, c), cols, rows);
            let (sa, _) = proj.project(pa);
            let (sb, _) = proj.project(pb);
            clip.line_segment([sa, sb], wire_stroke);
        }
    }

    paint_floor_grid(&proj, &clip, cols, rows);
    paint_axes(&proj, &clip, cols, rows);
    paint_active_marker(&proj, &clip, active_cell, cols, rows, z_norm);

    clip.text(
        rect.right_top() + Vec2::new(-6.0, 4.0),
        Align2::RIGHT_TOP,
        format!("{:.0}%", zoom * 100.0),
        FontId::monospace(9.0),
        MUTED,
    );
}

fn paint_floor_grid(proj: &MeshProjector, painter: &Painter, cols: usize, rows: usize) {
    let floor_z = display_z_norm(0.0);
    let stroke = Stroke::new(0.6, STROKE.gamma_multiply(0.55));
    for r in 0..rows {
        for c in 0..cols - 1 {
            let pa = proj.model_point(c as f32, r as f32, floor_z, cols, rows);
            let pb = proj.model_point((c + 1) as f32, r as f32, floor_z, cols, rows);
            let (sa, _) = proj.project(pa);
            let (sb, _) = proj.project(pb);
            painter.line_segment([sa, sb], stroke);
        }
    }
    for c in 0..cols {
        for r in 0..rows - 1 {
            let pa = proj.model_point(c as f32, r as f32, floor_z, cols, rows);
            let pb = proj.model_point(c as f32, (r + 1) as f32, floor_z, cols, rows);
            let (sa, _) = proj.project(pa);
            let (sb, _) = proj.project(pb);
            painter.line_segment([sa, sb], stroke);
        }
    }
}

fn paint_axis_label(painter: &Painter, anchor: Pos2, label: &str, color: Color32) {
    let font = FontId::proportional(10.0);
    let galley = painter.layout_no_wrap(label.into(), font, color);
    let size = galley.size();
    let rect = Rect::from_min_size(anchor, size).expand(3.0);
    painter.rect_filled(rect, 2.0, BG_FILL.gamma_multiply(0.92));
    painter.rect_stroke(rect, 2.0, Stroke::new(1.0, STROKE.gamma_multiply(0.8)));
    painter.galley(anchor, galley, color);
}

fn paint_axes(proj: &MeshProjector, painter: &Painter, cols: usize, rows: usize) {
    let origin = proj.model_point(0.0, 0.0, display_z_norm(0.0), cols, rows);
    let x_len = cols as f32 * proj.scale_xy * 0.5;
    let y_len = rows as f32 * proj.scale_xy * 0.5;
    let z_len = proj.scale_z * 0.85;

    let o = origin;
    let x_end = o.add(Vec3::new(x_len, 0.0, 0.0));
    let y_end = o.add(Vec3::new(0.0, y_len, 0.0));
    let z_end = o.add(Vec3::new(0.0, 0.0, z_len));

    let draw_axis = |from: Vec3, to: Vec3, color: Color32, label: &str| {
        let (a, _) = proj.project(from);
        let (b, _) = proj.project(to);
        painter.line_segment([a, b], Stroke::new(1.75, color));
        paint_axis_label(painter, b + Vec2::new(3.0, -3.0), label, color);
    };

    draw_axis(o, x_end, ACCENT_CYAN, "RPM");
    draw_axis(o, y_end, Color32::from_rgb(0xF5, 0x9E, 0x0B), "MAP");
    draw_axis(o, z_end, Color32::from_rgb(0x2F, 0x81, 0xF7), "Value");

    let (o_screen, _) = proj.project(o);
    painter.circle_filled(o_screen, 2.5, TEXT);
}

fn paint_active_marker(
    proj: &MeshProjector,
    painter: &Painter,
    active_cell: (usize, usize),
    cols: usize,
    rows: usize,
    z_norm: impl Fn(usize, usize) -> f32,
) {
    let (r, c) = active_cell;
    if r >= rows || c >= cols {
        return;
    }
    let p = proj.model_point(c as f32, r as f32, z_norm(r, c), cols, rows);
    let p_base = proj.model_point(c as f32, r as f32, display_z_norm(0.0), cols, rows);
    let (pos, _) = proj.project(p);
    let (base, _) = proj.project(p_base);
    painter.line_segment(
        [base, pos],
        Stroke::new(1.25, ACCENT_CYAN.gamma_multiply(0.45)),
    );
    painter.circle_filled(pos, 4.5, ACCENT_CYAN);
    painter.circle_stroke(pos, 7.0, Stroke::new(2.0, TEXT));
    painter.circle_stroke(pos, 11.0, Stroke::new(1.0, ACCENT_CYAN.gamma_multiply(0.35)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projector_depth_orders_near_before_far() {
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(200.0, 200.0));
        let proj = MeshProjector::new(rect, 4, 4, 0.8, 0.5, 1.0);
        let near = proj.model_point(2.0, 2.0, display_z_norm(0.9), 4, 4);
        let far = proj.model_point(2.0, 2.0, display_z_norm(0.1), 4, 4);
        let (_, d_near) = proj.project(near);
        let (_, d_far) = proj.project(far);
        assert!(d_near < d_far, "higher Z norm should be closer to camera");
    }

    #[test]
    fn backface_culled_when_facing_away() {
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(160.0, 160.0));
        let proj = MeshProjector::new(rect, 3, 3, 0.0, 1.2, 1.0);
        let p0 = proj.model_point(0.0, 0.0, 0.5, 3, 3);
        let p1 = proj.model_point(1.0, 0.0, 0.5, 3, 3);
        let p2 = proj.model_point(0.0, 1.0, 0.5, 3, 3);
        let n = proj.face_normal(p0, p1, p2);
        assert!(n.dot(proj.view_dir) > 0.0);
    }

    #[test]
    fn display_z_norm_softens_extremes() {
        assert!(display_z_norm(1.0) < 1.0);
        assert!(display_z_norm(0.0) > 0.0);
    }
}
