//! Rectangular cell selection, block edit operations, and undo/redo for calibration tables.
//! Pure functions over `Vec<Vec<f64>>` so every operation is unit-tested without the UI.

/// Inclusive rectangle spanned by an anchor and the cursor (either corner order).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellSelection {
    pub anchor: (usize, usize),
    pub cursor: (usize, usize),
}

impl CellSelection {
    pub fn single(cell: (usize, usize)) -> Self {
        Self { anchor: cell, cursor: cell }
    }

    /// `(row0, row1, col0, col1)`, all inclusive.
    pub fn bounds(&self) -> (usize, usize, usize, usize) {
        (
            self.anchor.0.min(self.cursor.0),
            self.anchor.0.max(self.cursor.0),
            self.anchor.1.min(self.cursor.1),
            self.anchor.1.max(self.cursor.1),
        )
    }

    pub fn contains(&self, cell: (usize, usize)) -> bool {
        let (r0, r1, c0, c1) = self.bounds();
        (r0..=r1).contains(&cell.0) && (c0..=c1).contains(&cell.1)
    }

    pub fn is_single(&self) -> bool {
        self.anchor == self.cursor
    }

    pub fn count(&self) -> usize {
        let (r0, r1, c0, c1) = self.bounds();
        (r1 - r0 + 1) * (c1 - c0 + 1)
    }

    /// Clamp to a `rows x cols` table (after a table switch or resize).
    pub fn clamped(&self, rows: usize, cols: usize) -> Self {
        let cl = |(r, c): (usize, usize)| (r.min(rows.saturating_sub(1)), c.min(cols.saturating_sub(1)));
        Self { anchor: cl(self.anchor), cursor: cl(self.cursor) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MathOp {
    Add(f64),
    Sub(f64),
    /// Change by a percentage: `+5` => x * 1.05, `-5` => x * 0.95.
    Percent(f64),
    Set(f64),
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn in_table(values: &[Vec<f64>], sel: &CellSelection) -> bool {
    let (_, r1, _, c1) = sel.bounds();
    r1 < values.len() && values.iter().all(|row| c1 < row.len())
}

pub fn apply_math(values: &mut [Vec<f64>], sel: &CellSelection, op: MathOp) {
    if !in_table(values, sel) {
        return;
    }
    let (r0, r1, c0, c1) = sel.bounds();
    for row in &mut values[r0..=r1] {
        for v in &mut row[c0..=c1] {
            *v = round2(match op {
                MathOp::Add(a) => *v + a,
                MathOp::Sub(a) => *v - a,
                MathOp::Percent(p) => *v * (1.0 + p / 100.0),
                MathOp::Set(a) => a,
            });
        }
    }
}

/// Linear fill along each row of the selection between its left and right edge cells.
pub fn interpolate_horizontal(values: &mut [Vec<f64>], sel: &CellSelection) {
    if !in_table(values, sel) {
        return;
    }
    let (r0, r1, c0, c1) = sel.bounds();
    if c1 <= c0 {
        return;
    }
    for row in &mut values[r0..=r1] {
        let (a, b) = (row[c0], row[c1]);
        for c in c0..=c1 {
            let t = (c - c0) as f64 / (c1 - c0) as f64;
            row[c] = round2(a + (b - a) * t);
        }
    }
}

/// Linear fill down each column of the selection between its top and bottom edge cells.
pub fn interpolate_vertical(values: &mut [Vec<f64>], sel: &CellSelection) {
    if !in_table(values, sel) {
        return;
    }
    let (r0, r1, c0, c1) = sel.bounds();
    if r1 <= r0 {
        return;
    }
    for c in c0..=c1 {
        let (a, b) = (values[r0][c], values[r1][c]);
        for r in r0..=r1 {
            let t = (r - r0) as f64 / (r1 - r0) as f64;
            values[r][c] = round2(a + (b - a) * t);
        }
    }
}

/// Bilinear fill of the selection from its four corner cells.
pub fn interpolate_2d(values: &mut [Vec<f64>], sel: &CellSelection) {
    if !in_table(values, sel) {
        return;
    }
    let (r0, r1, c0, c1) = sel.bounds();
    if r1 <= r0 || c1 <= c0 {
        // Degenerate rectangle: fall back to the 1D fill along its length.
        interpolate_horizontal(values, sel);
        interpolate_vertical(values, sel);
        return;
    }
    let (tl, tr, bl, br) = (values[r0][c0], values[r0][c1], values[r1][c0], values[r1][c1]);
    for r in r0..=r1 {
        let v = (r - r0) as f64 / (r1 - r0) as f64;
        for c in c0..=c1 {
            let u = (c - c0) as f64 / (c1 - c0) as f64;
            let top = tl + (tr - tl) * u;
            let bot = bl + (br - bl) * u;
            values[r][c] = round2(top + (bot - top) * v);
        }
    }
}

/// 3x3 weighted smooth (centre x4, edge neighbours x1) of the selected cells, reading
/// neighbours from the whole table so block edges blend into their surroundings.
/// Table-edge cells use only the neighbours that exist.
pub fn smooth(values: &mut [Vec<f64>], sel: &CellSelection) {
    if !in_table(values, sel) {
        return;
    }
    let rows = values.len();
    let cols = values[0].len();
    let src: Vec<Vec<f64>> = values.to_vec();
    let (r0, r1, c0, c1) = sel.bounds();
    for r in r0..=r1 {
        for c in c0..=c1 {
            let mut sum = src[r][c] * 4.0;
            let mut w = 4.0;
            let mut add = |rr: isize, cc: isize| {
                if rr >= 0 && cc >= 0 && (rr as usize) < rows && (cc as usize) < cols {
                    sum += src[rr as usize][cc as usize];
                    w += 1.0;
                }
            };
            let (ri, ci) = (r as isize, c as isize);
            add(ri - 1, ci);
            add(ri + 1, ci);
            add(ri, ci - 1);
            add(ri, ci + 1);
            values[r][c] = round2(sum / w);
        }
    }
}

/// Per-table snapshot history. A snapshot is the table's values *before* an edit.
#[derive(Default)]
pub struct UndoStack {
    undo: Vec<(usize, Vec<Vec<f64>>, String)>,
    redo: Vec<(usize, Vec<Vec<f64>>, String)>,
}

const MAX_UNDO: usize = 200;

impl UndoStack {
    /// Record `before` for `table_idx` ahead of an edit described by `label`.
    pub fn record(&mut self, table_idx: usize, before: Vec<Vec<f64>>, label: &str) {
        self.undo.push((table_idx, before, label.to_string()));
        if self.undo.len() > MAX_UNDO {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Pop the last edit. `current_of(idx)` supplies the table's present values (pushed to redo).
    /// Returns `(table_idx, values_to_restore, label)`.
    pub fn undo(
        &mut self,
        current_of: impl Fn(usize) -> Vec<Vec<f64>>,
    ) -> Option<(usize, Vec<Vec<f64>>, String)> {
        let (idx, before, label) = self.undo.pop()?;
        self.redo.push((idx, current_of(idx), label.clone()));
        Some((idx, before, label))
    }

    pub fn redo(
        &mut self,
        current_of: impl Fn(usize) -> Vec<Vec<f64>>,
    ) -> Option<(usize, Vec<Vec<f64>>, String)> {
        let (idx, after, label) = self.redo.pop()?;
        self.undo.push((idx, current_of(idx), label.clone()));
        Some((idx, after, label))
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|(_, _, l)| l.as_str())
    }

    /// Drop all history (table indices are meaningless after loading a new XDF/ROM).
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> Vec<Vec<f64>> {
        vec![
            vec![10.0, 20.0, 30.0, 40.0],
            vec![10.0, 99.0, 99.0, 40.0],
            vec![50.0, 60.0, 70.0, 80.0],
        ]
    }

    #[test]
    fn selection_bounds_any_corner_order() {
        let s = CellSelection { anchor: (2, 3), cursor: (0, 1) };
        assert_eq!(s.bounds(), (0, 2, 1, 3));
        assert_eq!(s.count(), 9);
        assert!(s.contains((1, 2)));
        assert!(!s.contains((1, 0)));
    }

    #[test]
    fn math_only_touches_selection() {
        let mut v = grid();
        apply_math(&mut v, &CellSelection { anchor: (0, 0), cursor: (0, 1) }, MathOp::Percent(10.0));
        assert_eq!(v[0], vec![11.0, 22.0, 30.0, 40.0]);
        assert_eq!(v[1][0], 10.0);
    }

    #[test]
    fn percent_is_change_not_fraction() {
        // Regression: '%' used to compute value * step/100 (80 -> 4.0 with step 5).
        let mut v = vec![vec![80.0]];
        apply_math(&mut v, &CellSelection::single((0, 0)), MathOp::Percent(5.0));
        assert_eq!(v[0][0], 84.0);
        apply_math(&mut v, &CellSelection::single((0, 0)), MathOp::Percent(-5.0));
        assert_eq!(v[0][0], 79.8);
    }

    #[test]
    fn horizontal_and_vertical_fill() {
        let mut v = grid();
        interpolate_horizontal(&mut v, &CellSelection { anchor: (1, 0), cursor: (1, 3) });
        assert_eq!(v[1], vec![10.0, 20.0, 30.0, 40.0]);
        let mut v = grid();
        interpolate_vertical(&mut v, &CellSelection { anchor: (0, 1), cursor: (2, 1) });
        assert_eq!([v[0][1], v[1][1], v[2][1]], [20.0, 40.0, 60.0]);
    }

    #[test]
    fn bilinear_fill_from_corners() {
        let mut v = grid();
        interpolate_2d(&mut v, &CellSelection { anchor: (0, 0), cursor: (2, 3) });
        // Corners kept; centre row is the average of top and bottom rows.
        assert_eq!(v[0], vec![10.0, 20.0, 30.0, 40.0]);
        assert_eq!(v[2], vec![50.0, 60.0, 70.0, 80.0]);
        assert_eq!(v[1], vec![30.0, 40.0, 50.0, 60.0]);
    }

    #[test]
    fn smooth_pulls_spike_toward_neighbours_and_handles_edges() {
        let mut v = grid();
        smooth(&mut v, &CellSelection { anchor: (0, 0), cursor: (2, 3) });
        assert!(v[1][1] < 99.0 && v[1][1] > 40.0);
        // Corner cell (2 neighbours) stays finite and near its region.
        assert!(v[0][0] > 9.0 && v[0][0] < 40.0);
    }

    #[test]
    fn out_of_bounds_selection_is_ignored() {
        let mut v = grid();
        apply_math(&mut v, &CellSelection { anchor: (0, 0), cursor: (5, 5) }, MathOp::Set(0.0));
        assert_eq!(v, grid());
    }

    #[test]
    fn undo_redo_roundtrip() {
        let mut tables = vec![grid()];
        let mut stack = UndoStack::default();
        stack.record(0, tables[0].clone(), "+5");
        apply_math(&mut tables[0], &CellSelection::single((0, 0)), MathOp::Add(5.0));
        assert_eq!(tables[0][0][0], 15.0);

        let snapshot = tables.clone();
        let (idx, vals, label) = stack.undo(|i| snapshot[i].clone()).unwrap();
        assert_eq!(label, "+5");
        tables[idx] = vals;
        assert_eq!(tables[0][0][0], 10.0);

        let snapshot = tables.clone();
        let (idx, vals, _) = stack.redo(|i| snapshot[i].clone()).unwrap();
        tables[idx] = vals;
        assert_eq!(tables[0][0][0], 15.0);
        assert!(stack.can_undo() && !stack.can_redo());
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut stack = UndoStack::default();
        stack.record(0, grid(), "a");
        stack.undo(|_| grid());
        assert!(stack.can_redo());
        stack.record(0, grid(), "b");
        assert!(!stack.can_redo());
    }
}
