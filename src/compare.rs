//! Tune compare (HP Tuners "Compare" / VF "compare files"): decode a second ROM with the same
//! definition and list every table and constant that differs from the current tune.

use crate::definitions::XdfDocument;

/// Values below this are treated as equal (display rounding / float noise).
const EPS: f64 = 1e-6;

#[derive(Clone, Debug, PartialEq)]
pub struct TableDiff {
    pub table_idx: usize,
    pub title: String,
    pub changed_cells: usize,
    pub total_cells: usize,
    /// Largest |current - compare| over the table.
    pub max_abs_delta: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConstantDiff {
    pub idx: usize,
    pub title: String,
    pub current: f64,
    pub compare: f64,
}

/// A second tune decoded with the current definition.
#[derive(Clone, Debug)]
pub struct CompareFile {
    pub name: String,
    pub tables: Vec<Vec<Vec<f64>>>,
    pub constants: Vec<f64>,
}

impl CompareFile {
    /// Decode `rom` (already placed in a buffer at `origin`) with a copy of `definition`.
    pub fn decode(name: &str, definition: &XdfDocument, rom: &[u8], origin: i64) -> Self {
        let mut doc = definition.clone();
        doc.bind_from_rom(rom, origin);
        Self {
            name: name.to_string(),
            tables: doc.tables.iter().map(|t| t.values.clone()).collect(),
            constants: doc.constants.iter().map(|c| c.value).collect(),
        }
    }
}

pub fn table_diff(current: &[Vec<f64>], compare: &[Vec<f64>]) -> (usize, usize, f64) {
    let mut changed = 0;
    let mut total = 0;
    let mut max_abs: f64 = 0.0;
    for (r, row) in current.iter().enumerate() {
        for (c, v) in row.iter().enumerate() {
            total += 1;
            let Some(o) = compare.get(r).and_then(|cr| cr.get(c)) else {
                changed += 1;
                continue;
            };
            let d = (v - o).abs();
            if d > EPS {
                changed += 1;
                max_abs = max_abs.max(d);
            }
        }
    }
    (changed, total, max_abs)
}

/// Every table and constant in `current` that differs from `compare`, in definition order.
pub fn diff(current: &XdfDocument, compare: &CompareFile) -> (Vec<TableDiff>, Vec<ConstantDiff>) {
    let tables = current
        .tables
        .iter()
        .enumerate()
        .filter_map(|(i, t)| {
            let other = compare.tables.get(i)?;
            let (changed, total, max_abs) = table_diff(&t.values, other);
            (changed > 0).then(|| TableDiff {
                table_idx: i,
                title: t.title.clone(),
                changed_cells: changed,
                total_cells: total,
                max_abs_delta: max_abs,
            })
        })
        .collect();
    let constants = current
        .constants
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let other = *compare.constants.get(i)?;
            ((c.value - other).abs() > EPS).then(|| ConstantDiff {
                idx: i,
                title: c.title.clone(),
                current: c.value,
                compare: other,
            })
        })
        .collect();
    (tables, constants)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc_with_rom(rom: &[u8]) -> XdfDocument {
        let xml = std::fs::read_to_string("tests/fixtures/tacoma-bench-table.xdf").unwrap();
        let mut d = XdfDocument::parse(&xml).unwrap();
        d.bind_from_rom(rom, 0);
        d
    }

    #[test]
    fn identical_tunes_have_no_diffs() {
        let rom = vec![10u8; 0x200];
        let cur = doc_with_rom(&rom);
        let cmp = CompareFile::decode("same.bin", &cur, &rom, 0);
        let (t, c) = diff(&cur, &cmp);
        assert!(t.is_empty() && c.is_empty());
    }

    #[test]
    fn reports_changed_cells_and_constants() {
        let stock = vec![10u8; 0x200];
        let mut modded = stock.clone();
        modded[0x100] = 14; // one VE cell
        modded[0x120] = 200; // rev limit constant (X * 32)
        let cur = doc_with_rom(&modded);
        let cmp = CompareFile::decode("stock.bin", &cur, &stock, 0);
        let (t, c) = diff(&cur, &cmp);
        assert_eq!(t.len(), 1);
        assert_eq!((t[0].changed_cells, t[0].total_cells), (1, 4));
        assert_eq!(t[0].max_abs_delta, 4.0);
        assert_eq!(c.len(), 1);
        assert_eq!((c[0].current, c[0].compare), (6400.0, 320.0));
    }
}
