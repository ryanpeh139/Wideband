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

}

struct MeshProjector {
    center: Pos2,
    scale_xy: f32,
    scale_z: f32,
    yaw: f32,
    pitch: f32,
    eye: Vec3,
}

impl MeshProjector {
    fn new(rect: Rect, rows: usize, cols: usize, yaw: f32, pitch: f32) -> Self {
        let aspect = rect.width() / rect.height().max(1.0);
        let span = (rows.max(cols) as f32).max(1.0);
        let fit = rect.width().min(rect.height()) * 0.38;
        let scale_xy = fit / span;
        let scale_z = fit * 0.55;

        // Slightly widen X when the viewport is landscape so RPM axis uses horizontal space.
        let scale_xy = if aspect > 1.15 {
            scale_xy * aspect.sqrt().min(1.35)
        } else {
            scale_xy
        };

        let cos_p = pitch.cos();
        let cos_y = yaw.cos();
        let sin_y = yaw.sin();

        // Camera eye in model space (used for depth sorting).
        let eye = Vec3::new(
            -sin_y * 2.2 * scale_xy,
            -cos_y * 2.2 * scale_xy,
            1.8 * scale_z * cos_p + 2.0 * scale_z,
        );

        Self {
            center: rect.center(),
            scale_xy,
            scale_z,
            yaw,
            pitch,
            eye,
        }
    }

    /// Model axes: X = RPM (columns), Y = MAP / load (rows), Z = table value.
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

    fn project(&self, p: Vec3) -> (Pos2, f32) {
        let r = self.rotate(p);
        let depth = (r.x - self.eye.x).hypot(r.y - self.eye.y) + (r.z - self.eye.z) * 0.35;
        let screen = Pos2::new(self.center.x + r.x, self.center.y - r.z);
        (screen, depth)
    }
}

enum DrawCmd {
    FillTriangle {
        depth: f32,
        pts: [Pos2; 3],
        fill: Color32,
        stroke: Stroke,
    },
    Line {
        depth: f32,
        a: Pos2,
        b: Pos2,
        stroke: Stroke,
    },
}

/// Paint the 3D surface mesh into `rect` using the full available canvas size.
pub fn paint_surface_mesh(
    painter: &Painter,
    rect: Rect,
    table: &XdfTable,
    yaw: f32,
    pitch: f32,
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

    let proj = MeshProjector::new(rect, rows, cols, yaw, pitch);
    let mut cmds: Vec<DrawCmd> = Vec::new();

    let z_norm = |r: usize, c: usize| -> f32 {
        let val = table.values[r][c];
        if max_val > min_val {
            ((val - min_val) / (max_val - min_val)) as f32
        } else {
            0.5
        }
    };

    let mut push_tri = |r0: usize, c0: usize, r1: usize, c1: usize, r2: usize, c2: usize| {
        let p0 = proj.model_point(c0 as f32, r0 as f32, z_norm(r0, c0), cols, rows);
        let p1 = proj.model_point(c1 as f32, r1 as f32, z_norm(r1, c1), cols, rows);
        let p2 = proj.model_point(c2 as f32, r2 as f32, z_norm(r2, c2), cols, rows);
        let (s0, d0) = proj.project(p0);
        let (s1, d1) = proj.project(p1);
        let (s2, d2) = proj.project(p2);
        let depth = (d0 + d1 + d2) / 3.0;
        let mid_val = (table.values[r0][c0] + table.values[r1][c1] + table.values[r2][c2]) / 3.0;
        let fill = heatmap_color(mid_val, min_val, max_val).gamma_multiply(0.82);
        cmds.push(DrawCmd::FillTriangle {
            depth,
            pts: [s0, s1, s2],
            fill,
            stroke: Stroke::new(0.75, Color32::from_rgba_unmultiplied(12, 16, 22, 180)),
        });
    };

    for r in 0..rows - 1 {
        for c in 0..cols - 1 {
            push_tri(r, c, r + 1, c, r + 1, c + 1);
            push_tri(r, c, r + 1, c + 1, r, c + 1);
        }
    }

    // Wireframe grid (segment list, sorted back-to-front).
    let wire_stroke = Stroke::new(0.85, STROKE.gamma_multiply(0.9));
    let mut push_line = |ra: usize, ca: usize, rb: usize, cb: usize| {
        let pa = proj.model_point(ca as f32, ra as f32, z_norm(ra, ca), cols, rows);
        let pb = proj.model_point(cb as f32, rb as f32, z_norm(rb, cb), cols, rows);
        let (sa, da) = proj.project(pa);
        let (sb, db) = proj.project(pb);
        cmds.push(DrawCmd::Line {
            depth: (da + db) * 0.5,
            a: sa,
            b: sb,
            stroke: wire_stroke,
        });
    };

    for r in 0..rows {
        for c in 0..cols - 1 {
            push_line(r, c, r, c + 1);
        }
    }
    for c in 0..cols {
        for r in 0..rows - 1 {
            push_line(r, c, r + 1, c);
        }
    }

    // Painter's algorithm: far geometry first.
    cmds.sort_by(|a, b| {
        let da = match a {
            DrawCmd::FillTriangle { depth, .. } | DrawCmd::Line { depth, .. } => depth,
        };
        let db = match b {
            DrawCmd::FillTriangle { depth, .. } | DrawCmd::Line { depth, .. } => depth,
        };
        db.partial_cmp(&da).unwrap_or(std::cmp::Ordering::Equal)
    });

    for cmd in cmds {
        match cmd {
            DrawCmd::FillTriangle { pts, fill, stroke, .. } => {
                painter.add(Shape::convex_polygon(pts.to_vec(), fill, stroke));
            }
            DrawCmd::Line { a, b, stroke, .. } => {
                painter.line_segment([a, b], stroke);
            }
        }
    }

    paint_axes(&proj, painter, cols, rows);
    paint_active_marker(&proj, painter, active_cell, cols, rows, z_norm);
}

fn paint_axes(proj: &MeshProjector, painter: &Painter, cols: usize, rows: usize) {
    let origin = proj.model_point(0.0, 0.0, 0.0, cols, rows);
    let x_len = cols as f32 * proj.scale_xy * 0.55;
    let y_len = rows as f32 * proj.scale_xy * 0.55;
    let z_len = proj.scale_z * 0.85;

    let o = origin;
    let x_end = o.add(Vec3::new(x_len, 0.0, 0.0));
    let y_end = o.add(Vec3::new(0.0, y_len, 0.0));
    let z_end = o.add(Vec3::new(0.0, 0.0, z_len));

    let axis_stroke = |color: Color32| Stroke::new(1.5, color);

    let draw_axis = |from: Vec3, to: Vec3, color: Color32, label: &str| {
        let (a, _) = proj.project(from);
        let (b, _) = proj.project(to);
        painter.line_segment([a, b], axis_stroke(color));
        painter.text(
            b + Vec2::new(4.0, -2.0),
            Align2::LEFT_BOTTOM,
            label,
            FontId::proportional(10.0),
            color,
        );
    };

    draw_axis(o, x_end, ACCENT_CYAN, "X RPM");
    draw_axis(o, y_end, Color32::from_rgb(0xF5, 0x9E, 0x0B), "Y MAP");
    draw_axis(o, z_end, Color32::from_rgb(0x2F, 0x81, 0xF7), "Z Value");

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
    let (pos, _) = proj.project(p);
    painter.circle_filled(pos, 5.0, ACCENT_CYAN);
    painter.circle_stroke(pos, 8.0, Stroke::new(2.0, TEXT));
}
