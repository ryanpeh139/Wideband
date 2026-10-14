pub use crate::rom_image::CuwFile;
use crate::security_config::locate;
use crate::strategy_meta::{enrich_document_strategy_meta, parse_wideband_meta, WidebandMeta};
use roxmltree::Document;

pub use crate::rom_image::{RomImage, MAX_CUW_SPAN};

// ---------------------------------------------------------------------------
// ScaleEquation Math Parser & Inverter
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub enum MathExpr {
    Const(f64),
    Variable,
    UnaryNeg(Box<MathExpr>),
    Binary(char, Box<MathExpr>, Box<MathExpr>),
}

impl MathExpr {
    pub fn eval(&self, x: f64) -> f64 {
        match self {
            MathExpr::Const(c) => *c,
            MathExpr::Variable => x,
            MathExpr::UnaryNeg(inner) => -inner.eval(x),
            MathExpr::Binary(op, left, right) => {
                let a = left.eval(x);
                let b = right.eval(x);
                match op {
                    '+' => a + b,
                    '-' => a - b,
                    '*' => a * b,
                    '/' => {
                        if b == 0.0 {
                            0.0
                        } else {
                            a / b
                        }
                    }
                    '%' => {
                        if b == 0.0 {
                            0.0
                        } else {
                            a % b
                        }
                    }
                    '&' => ((a as i64) & (b as i64)) as f64,
                    '|' => ((a as i64) | (b as i64)) as f64,
                    '^' => ((a as i64) ^ (b as i64)) as f64,
                    _ => a,
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct ScaleEquation {
    pub raw_str: String,
    expr: MathExpr,
}

impl ScaleEquation {
    pub fn parse(eq_str: &str) -> Self {
        let trimmed = eq_str.trim();
        if trimmed.is_empty() {
            return Self {
                raw_str: "X".to_string(),
                expr: MathExpr::Variable,
            };
        }

        let expr = Self::parse_expression(trimmed).unwrap_or(MathExpr::Variable);
        Self {
            raw_str: trimmed.to_string(),
            expr,
        }
    }

    fn parse_expression(s: &str) -> Option<MathExpr> {
        let tokens: Vec<char> = s.chars().filter(|c| !c.is_whitespace()).collect();
        let mut idx = 0;
        Self::parse_add_sub(&tokens, &mut idx)
    }

    fn parse_add_sub(tokens: &[char], idx: &mut usize) -> Option<MathExpr> {
        let mut left = Self::parse_mul_div(tokens, idx)?;
        while *idx < tokens.len() {
            let op = tokens[*idx];
            if op == '+' || op == '-' {
                *idx += 1;
                let right = Self::parse_mul_div(tokens, idx)?;
                left = MathExpr::Binary(op, Box::new(left), Box::new(right));
            } else {
                break;
            }
        }
        Some(left)
    }

    fn parse_mul_div(tokens: &[char], idx: &mut usize) -> Option<MathExpr> {
        let mut left = Self::parse_unary(tokens, idx)?;
        while *idx < tokens.len() {
            let op = tokens[*idx];
            if op == '*' || op == '/' || op == '%' {
                *idx += 1;
                let right = Self::parse_unary(tokens, idx)?;
                left = MathExpr::Binary(op, Box::new(left), Box::new(right));
            } else {
                break;
            }
        }
        Some(left)
    }

    fn parse_unary(tokens: &[char], idx: &mut usize) -> Option<MathExpr> {
        if *idx < tokens.len() && tokens[*idx] == '-' {
            *idx += 1;
            let inner = Self::parse_primary(tokens, idx)?;
            return Some(MathExpr::UnaryNeg(Box::new(inner)));
        }
        Self::parse_primary(tokens, idx)
    }

    fn parse_primary(tokens: &[char], idx: &mut usize) -> Option<MathExpr> {
        if *idx >= tokens.len() {
            return None;
        }

        if tokens[*idx] == '(' {
            *idx += 1;
            let inner = Self::parse_add_sub(tokens, idx)?;
            if *idx < tokens.len() && tokens[*idx] == ')' {
                *idx += 1;
            }
            return Some(inner);
        }

        if tokens[*idx] == 'X' || tokens[*idx] == 'x' {
            *idx += 1;
            return Some(MathExpr::Variable);
        }

        // Parse number
        let mut num_str = String::new();
        while *idx < tokens.len() && (tokens[*idx].is_ascii_digit() || tokens[*idx] == '.') {
            num_str.push(tokens[*idx]);
            *idx += 1;
        }

        if let Ok(val) = num_str.parse::<f64>() {
            Some(MathExpr::Const(val))
        } else {
            Some(MathExpr::Variable)
        }
    }

    pub fn evaluate(&self, x: f64) -> f64 {
        self.expr.eval(x)
    }

    /// Invert engineering value back to raw byte/integer
    pub fn invert(&self, eng: f64) -> f64 {
        // Linear bisection search for monotonic or piecewise scaling
        let mut low = -1_000_000.0;
        let mut high = 1_000_000.0;
        let f_low = self.evaluate(low);
        let f_high = self.evaluate(high);

        let increasing = f_high >= f_low;
        for _ in 0..40 {
            let mid = (low + high) * 0.5;
            let val = self.evaluate(mid);
            if (val - eng).abs() < 1e-6 {
                return mid;
            }
            if increasing {
                if val < eng {
                    low = mid;
                } else {
                    high = mid;
                }
            } else {
                if val > eng {
                    low = mid;
                } else {
                    high = mid;
                }
            }
        }
        (low + high) * 0.5
    }
}

// ---------------------------------------------------------------------------
// TunerPro XDF Document Parser (<XDFTABLE>, <XDFCONSTANT>, <XDFFLAG>)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct XdfAxis {
    pub id: char, // 'x', 'y', 'z'
    pub dim: usize,
    pub address: u32,
    pub units: String,
    pub equation: ScaleEquation,
    pub labels: Vec<f64>,
    /// Breakpoints live in the ROM at `address` (XDF EMBEDDEDDATA, no static LABELs):
    /// re-read on every ROM bind so a newly opened ROM shows its own RPM/load axes.
    pub rom_backed: bool,
}

#[derive(Clone, Debug)]
pub struct XdfTable {
    pub id: String,
    pub title: String,
    pub description: String,
    pub wideband_meta: WidebandMeta,
    pub address: u32,
    pub x_axis: XdfAxis,
    pub y_axis: XdfAxis,
    pub z_equation: ScaleEquation,
    pub z_units: String,
    pub element_bits: u8,
    pub values: Vec<Vec<f64>>,
}

#[derive(Clone, Debug)]
pub struct XdfConstant {
    pub id: String,
    pub title: String,
    pub wideband_meta: WidebandMeta,
    pub address: u32,
    pub units: String,
    pub equation: ScaleEquation,
    pub value: f64,
}

#[derive(Clone, Debug)]
pub struct XdfFlag {
    pub id: String,
    pub title: String,
    pub description: String,
    pub wideband_meta: WidebandMeta,
    pub address: u32,
    pub mask: u8,
    pub is_set: bool,
}

#[derive(Clone, Debug, Default)]
pub struct XdfDocument {
    pub title: String,
    pub tables: Vec<XdfTable>,
    pub constants: Vec<XdfConstant>,
    pub flags: Vec<XdfFlag>,
}

impl XdfDocument {
    pub fn parse(xml_content: &str) -> Result<Self, String> {
        let doc = Document::parse(xml_content).map_err(|e| format!("XML parse error: {}", e))?;
        let root = doc.root_element();

        let mut xdf = XdfDocument::default();

        if let Some(header) = root.children().find(|n| n.has_tag_name("XDFHEADER")) {
            if let Some(desc) = header.children().find(|n| n.has_tag_name("deftitle")) {
                xdf.title = desc.text().unwrap_or("TunerPro Definition").to_string();
            }
        }

        for node in root.children() {
            if node.has_tag_name("XDFTABLE") {
                if let Some(table) = Self::parse_table(&node) {
                    xdf.tables.push(table);
                }
            } else if node.has_tag_name("XDFCONSTANT") {
                if let Some(constant) = Self::parse_constant(&node) {
                    xdf.constants.push(constant);
                }
            } else if node.has_tag_name("XDFFLAG") {
                if let Some(flag) = Self::parse_flag(&node) {
                    xdf.flags.push(flag);
                }
            }
        }

        enrich_document_strategy_meta(&mut xdf);
        Ok(xdf)
    }

    /// Load engineering values from a ROM image using XDF addresses.
    pub fn bind_from_rom(&mut self, rom: &[u8], origin: i64) {
        for table in &mut self.tables {
            // ROM-backed axes are re-read on every bind; the old placeholder-only check meant the
            // startup 0xFF mock ROM froze axes at 255*scale and a real ROM never replaced them.
            for axis in [&mut table.x_axis, &mut table.y_axis] {
                if axis.rom_backed || labels_look_like_placeholders(&axis.labels) {
                    read_axis_labels_from_rom(rom, origin, axis);
                }
            }
            read_table_from_rom(rom, origin, table);
        }
        for constant in &mut self.constants {
            if let Some(v) = read_scalar_u8(rom, origin, constant.address, &constant.equation) {
                constant.value = v;
            }
        }
        for flag in &mut self.flags {
            if let Some(idx) = locate(rom, origin, flag.address as i64) {
                flag.is_set = (rom[idx] & flag.mask) != 0;
            }
        }
    }

    pub fn write_table_to_rom(&self, rom: &mut [u8], origin: i64, table_idx: usize) -> bool {
        if table_idx >= self.tables.len() {
            return false;
        }
        write_table_to_rom(rom, origin, &self.tables[table_idx])
    }

    /// Re-decode one table from the ROM, so the UI shows what was actually stored after the
    /// equation inverse + integer quantisation (not the unrounded edit value).
    pub fn reload_table_from_rom(&mut self, rom: &[u8], origin: i64, table_idx: usize) {
        if let Some(table) = self.tables.get_mut(table_idx) {
            read_table_from_rom(rom, origin, table);
        }
    }

    pub fn write_all_tables_to_rom(&self, rom: &mut [u8], origin: i64) {
        for table in &self.tables {
            write_table_to_rom(rom, origin, table);
        }
    }

    pub fn write_constant_to_rom(&self, rom: &mut [u8], origin: i64, constant: &XdfConstant) -> bool {
        write_scalar_u8(rom, origin, constant.address, &constant.equation, constant.value)
    }

    pub fn write_all_constants_to_rom(&self, rom: &mut [u8], origin: i64) {
        for constant in &self.constants {
            write_scalar_u8(rom, origin, constant.address, &constant.equation, constant.value);
        }
    }

    pub fn write_flag_to_rom(&self, rom: &mut [u8], origin: i64, flag: &XdfFlag) {
        if let Some(idx) = locate(rom, origin, flag.address as i64) {
            if flag.is_set {
                rom[idx] |= flag.mask;
            } else {
                rom[idx] &= !flag.mask;
            }
        }
    }

    fn parse_table(node: &roxmltree::Node) -> Option<XdfTable> {
        let id = node.attribute("uniqueid").unwrap_or("table").to_string();
        let title = node.children().find(|n| n.has_tag_name("title"))
            .and_then(|n| n.text()).unwrap_or("Calibration Table").to_string();
        let description = node.children().find(|n| n.has_tag_name("description"))
            .and_then(|n| n.text()).unwrap_or("").to_string();

        let mut x_axis = XdfAxis {
            id: 'x',
            dim: 16,
            address: 0,
            units: "RPM".to_string(),
            equation: ScaleEquation::parse("X"),
            labels: Vec::new(),
            rom_backed: false,
        };
        let mut y_axis = XdfAxis {
            id: 'y',
            dim: 12,
            address: 0,
            units: "MAP (kPa)".to_string(),
            equation: ScaleEquation::parse("X"),
            labels: Vec::new(),
            rom_backed: false,
        };
        let mut z_equation = ScaleEquation::parse("X");
        let mut z_units = "VE %".to_string();
        let mut table_addr = 0u32;
        let mut element_bits = 8u8;

        for axis_node in node.children().filter(|n| n.has_tag_name("XDFAXIS")) {
            let axis_id = axis_node.attribute("id").unwrap_or("x");
            let dim = axis_dimension(&axis_node);
            let units = axis_node.children().find(|n| n.has_tag_name("units"))
                .and_then(|n| n.text()).unwrap_or("").to_string();
            let eq_str = axis_node.children().find(|n| n.has_tag_name("MATH"))
                .and_then(|n| n.attribute("equation")).unwrap_or("X");

            let addr = embedded_address(&axis_node).unwrap_or(0);
            let labels = parse_axis_labels(&axis_node, dim);
            let has_static_labels = axis_node.children().any(|n| n.has_tag_name("LABEL"));

            let axis = XdfAxis {
                id: axis_id.chars().next().unwrap_or('x'),
                dim,
                address: addr,
                units: units.clone(),
                equation: ScaleEquation::parse(eq_str),
                labels,
                rom_backed: addr != 0 && !has_static_labels,
            };

            match axis_id {
                "x" | "X" => x_axis = axis,
                "y" | "Y" => y_axis = axis,
                "z" | "Z" => {
                    if table_addr == 0 {
                        table_addr = addr;
                    }
                    z_equation = ScaleEquation::parse(eq_str);
                    z_units = units.clone();
                    element_bits = Self::embedded_element_bits(&axis_node).unwrap_or(8);
                }
                _ => {}
            }
        }

        if table_addr == 0 {
            table_addr = embedded_address(node).unwrap_or(0);
        }

        let values = vec![vec![0.0; x_axis.dim]; y_axis.dim];
        let combined = format!("{} {}", title, description);
        let wideband_meta = parse_wideband_meta(&combined);

        Some(XdfTable {
            id,
            title,
            description,
            wideband_meta,
            address: table_addr,
            x_axis,
            y_axis,
            z_equation,
            z_units,
            element_bits,
            values,
        })
    }

    fn embedded_element_bits(axis_node: &roxmltree::Node) -> Option<u8> {
        axis_node
            .children()
            .find(|n| n.has_tag_name("EMBEDDEDDATA"))
            .and_then(|n| n.attribute("mmedelementsizebits"))
            .and_then(|t| t.parse().ok())
    }

    fn parse_constant(node: &roxmltree::Node) -> Option<XdfConstant> {
        let id = node.attribute("uniqueid").unwrap_or("const").to_string();
        let title = node.children().find(|n| n.has_tag_name("title"))
            .and_then(|n| n.text()).unwrap_or("Scalar Constant").to_string();
        let units = node.children().find(|n| n.has_tag_name("units"))
            .and_then(|n| n.text()).unwrap_or("").to_string();
        let eq_str = node.children().find(|n| n.has_tag_name("MATH"))
            .and_then(|n| n.attribute("equation")).unwrap_or("X");

        let address = node.children().find(|n| n.has_tag_name("EMBEDDEDDATA"))
            .and_then(|n| n.attribute("mmedaddress"))
            .and_then(|a| u32::from_str_radix(a.trim_start_matches("0x"), 16).ok())
            .unwrap_or(0);

        let wideband_meta = parse_wideband_meta(&title);

        Some(XdfConstant {
            id,
            title,
            wideband_meta,
            address,
            units,
            equation: ScaleEquation::parse(eq_str),
            value: 6500.0, // Default rev limiter scalar
        })
    }

    fn parse_flag(node: &roxmltree::Node) -> Option<XdfFlag> {
        let id = node.attribute("uniqueid").unwrap_or("flag").to_string();
        let title = node.children().find(|n| n.has_tag_name("title"))
            .and_then(|n| n.text()).unwrap_or("Bit Switch").to_string();
        let description = node.children().find(|n| n.has_tag_name("description"))
            .and_then(|n| n.text()).unwrap_or("").to_string();

        let mask = node.children().find(|n| n.has_tag_name("mask"))
            .and_then(|n| n.text())
            .and_then(|m| u8::from_str_radix(m.trim_start_matches("0x"), 16).ok())
            .unwrap_or(0x01);

        let address = node.children().find(|n| n.has_tag_name("EMBEDDEDDATA"))
            .and_then(|n| n.attribute("mmedaddress"))
            .and_then(|a| u32::from_str_radix(a.trim_start_matches("0x"), 16).ok())
            .unwrap_or(0);

        let combined = format!("{} {}", title, description);
        let wideband_meta = parse_wideband_meta(&combined);

        Some(XdfFlag {
            id,
            title,
            description,
            wideband_meta,
            address,
            mask,
            is_set: false,
        })
    }

    pub fn sample() -> Self {
        let mut xdf = Self::default();
        xdf.title = "Wideband Sample Motorsport Calibration".to_string();

        // 1. Primary Volumetric Efficiency (VE) Map
        let x_rpm: Vec<f64> = vec![800.0, 1200.0, 1600.0, 2000.0, 2400.0, 2800.0, 3200.0, 3600.0, 4000.0, 4400.0, 4800.0, 5200.0, 5600.0, 6000.0, 6400.0, 6800.0];
        let y_map: Vec<f64> = vec![20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0, 90.0, 100.0, 120.0, 140.0, 160.0];

        let mut ve_values = vec![vec![0.0; x_rpm.len()]; y_map.len()];
        for (r, &map) in y_map.iter().enumerate() {
            for (c, &rpm) in x_rpm.iter().enumerate() {
                // Realistic VE curve with torque peak around 4400 RPM
                let delta_rpm = (rpm - 4400.0) / 4400.0;
                let rpm_factor = 1.0 - delta_rpm * delta_rpm * 0.35;
                let map_factor = (map / 160.0).powf(0.65);
                ve_values[r][c] = (45.0 + 52.0 * rpm_factor * map_factor).round();
            }
        }

        xdf.tables.push(XdfTable {
            id: "ve_main".to_string(),
            title: "Volumetric Efficiency Table (Main)".to_string(),
            description: "Primary Speed-Density VE fuel delivery table vs RPM and MAP kPa. wb-meta: manager=load; role=table".to_string(),
            wideband_meta: WidebandMeta::default(),
            address: 0x10000,
            x_axis: XdfAxis {
                id: 'x',
                dim: x_rpm.len(),
                address: 0x10100,
                units: "RPM".to_string(),
                equation: ScaleEquation::parse("X"),
                labels: x_rpm,
                rom_backed: false,
            },
            y_axis: XdfAxis {
                id: 'y',
                dim: y_map.len(),
                address: 0x10180,
                units: "MAP (kPa)".to_string(),
                equation: ScaleEquation::parse("X"),
                labels: y_map,
                rom_backed: false,
            },
            z_equation: ScaleEquation::parse("X * 0.1"),
            z_units: "%".to_string(),
            element_bits: 8,
            values: ve_values,
        });

        // 2. Base Spark Timing Table
        let spark_rpm = vec![1000.0, 1500.0, 2000.0, 2500.0, 3000.0, 3500.0, 4000.0, 4500.0, 5000.0, 5500.0, 6000.0, 6500.0];
        let spark_load = vec![20.0, 40.0, 60.0, 80.0, 100.0, 120.0];
        let mut spark_values = vec![vec![0.0; spark_rpm.len()]; spark_load.len()];
        for (r, &load) in spark_load.iter().enumerate() {
            for (c, &rpm) in spark_rpm.iter().enumerate() {
                let timing: f64 = 42.0 - (load * 0.18) + (rpm * 0.002);
                spark_values[r][c] = timing.clamp(8.0, 48.0);
            }
        }

        xdf.tables.push(XdfTable {
            id: "spark_base".to_string(),
            title: "Base Spark Advance Table".to_string(),
            description: "Target ignition advance in degrees BTDC. wb-meta: manager=load; role=target; depends=ve_main".to_string(),
            wideband_meta: WidebandMeta::default(),
            address: 0x10800,
            x_axis: XdfAxis {
                id: 'x',
                dim: spark_rpm.len(),
                address: 0x10880,
                units: "RPM".to_string(),
                equation: ScaleEquation::parse("X"),
                labels: spark_rpm,
                rom_backed: false,
            },
            y_axis: XdfAxis {
                id: 'y',
                dim: spark_load.len(),
                address: 0x108C0,
                units: "Load %".to_string(),
                equation: ScaleEquation::parse("X"),
                labels: spark_load,
                rom_backed: false,
            },
            z_equation: ScaleEquation::parse("X * 0.25"),
            z_units: "deg BTDC".to_string(),
            element_bits: 8,
            values: spark_values,
        });

        // 3. Target Lambda / Open Loop AFR Table
        let lambda_rpm = vec![1000.0, 2000.0, 3000.0, 4000.0, 5000.0, 6000.0];
        let lambda_map = vec![40.0, 70.0, 100.0, 130.0, 160.0];
        let mut lambda_values = vec![vec![0.0; lambda_rpm.len()]; lambda_map.len()];
        for (r, &map) in lambda_map.iter().enumerate() {
            for (c, _) in lambda_rpm.iter().enumerate() {
                lambda_values[r][c] = if map <= 70.0 {
                    1.00 // Stoich idle/cruise
                } else if map <= 100.0 {
                    0.92 // High load NA
                } else {
                    0.82 // Boost WOT
                };
            }
        }

        xdf.tables.push(XdfTable {
            id: "target_lambda".to_string(),
            title: "Target Lambda (Power Enrichment)".to_string(),
            description: "Target equivalence ratio (Lambda). 1.0 = Stoichiometric. wb-meta: manager=load; role=target; depends=ve_main".to_string(),
            wideband_meta: WidebandMeta::default(),
            address: 0x11000,
            x_axis: XdfAxis {
                id: 'x',
                dim: lambda_rpm.len(),
                address: 0x11050,
                units: "RPM".to_string(),
                equation: ScaleEquation::parse("X"),
                labels: lambda_rpm,
                rom_backed: false,
            },
            y_axis: XdfAxis {
                id: 'y',
                dim: lambda_map.len(),
                address: 0x11080,
                units: "MAP (kPa)".to_string(),
                equation: ScaleEquation::parse("X"),
                labels: lambda_map,
                rom_backed: false,
            },
            z_equation: ScaleEquation::parse("X * 0.0078125"),
            z_units: "Lambda".to_string(),
            element_bits: 8,
            values: lambda_values,
        });

        // Constants
        xdf.constants.push(XdfConstant {
            id: "rev_limit".to_string(),
            title: "Hard Rev Limiter".to_string(),
            wideband_meta: WidebandMeta::default(),
            address: 0x12000,
            units: "RPM".to_string(),
            equation: ScaleEquation::parse("X"),
            value: 6500.0,
        });
        xdf.constants.push(XdfConstant {
            id: "speed_limiter".to_string(),
            title: "Vehicle Top Speed Limiter".to_string(),
            wideband_meta: WidebandMeta::default(),
            address: 0x12004,
            units: "MPH".to_string(),
            equation: ScaleEquation::parse("X"),
            value: 115.0,
        });

        // Flags
        xdf.flags.push(XdfFlag {
            id: "flag_vvt_enable".to_string(),
            title: "Variable Valve Timing (VVT-i) Active".to_string(),
            description: "Enables intake and exhaust camshaft phaser closed loop positioning. wb-meta: manager=vvt; role=logic".to_string(),
            wideband_meta: WidebandMeta::default(),
            address: 0x13000,
            mask: 0x01,
            is_set: true,
        });
        xdf.flags.push(XdfFlag {
            id: "flag_cat_overheat".to_string(),
            title: "Catalytic Converter Overtemp Protection (COT)".to_string(),
            description: "Enables fuel enrichment dumping when catalytic bed temp model exceeds 890 C.".to_string(),
            wideband_meta: WidebandMeta::default(),
            address: 0x13000,
            mask: 0x02,
            is_set: false,
        });
        xdf.flags.push(XdfFlag {
            id: "flag_launch_control".to_string(),
            title: "Motorsport Launch Control (2-Step)".to_string(),
            description: "Enables clutch/brake stationary RPM limiting. wb-meta: manager=boost; role=logic; depends=boost_target".to_string(),
            wideband_meta: WidebandMeta::default(),
            address: 0x13000,
            mask: 0x04,
            is_set: true,
        });

        // Boost manager demo tables (tagged via wb-meta in description)
        let boost_rpm = vec![2000.0, 3000.0, 4000.0, 5000.0, 6000.0];
        xdf.tables.push(XdfTable {
            id: "boost_target".to_string(),
            title: "Boost Target (kPa)".to_string(),
            description: "wb-meta: manager=boost; role=target; depends=ve_main,target_lambda".to_string(),
            wideband_meta: WidebandMeta::default(),
            address: 0x14000,
            x_axis: XdfAxis {
                id: 'x',
                dim: boost_rpm.len(),
                address: 0x14080,
                units: "RPM".to_string(),
                equation: ScaleEquation::parse("X"),
                labels: boost_rpm.clone(),
                rom_backed: false,
            },
            y_axis: XdfAxis {
                id: 'y',
                dim: 1,
                address: 0,
                units: "—".to_string(),
                equation: ScaleEquation::parse("X"),
                labels: vec![0.0],
                rom_backed: false,
            },
            z_equation: ScaleEquation::parse("X"),
            z_units: "kPa".to_string(),
            element_bits: 8,
            values: vec![vec![110.0, 130.0, 150.0, 160.0, 160.0]],
        });
        xdf.tables.push(XdfTable {
            id: "wg_duty".to_string(),
            title: "Wastegate Duty Cycle".to_string(),
            description: "wb-meta: manager=boost; role=actuator; depends=boost_target".to_string(),
            wideband_meta: WidebandMeta::default(),
            address: 0x14100,
            x_axis: XdfAxis {
                id: 'x',
                dim: boost_rpm.len(),
                address: 0x14180,
                units: "RPM".to_string(),
                equation: ScaleEquation::parse("X"),
                labels: boost_rpm,
                rom_backed: false,
            },
            y_axis: XdfAxis {
                id: 'y',
                dim: 1,
                address: 0,
                units: "—".to_string(),
                equation: ScaleEquation::parse("X"),
                labels: vec![0.0],
                rom_backed: false,
            },
            z_equation: ScaleEquation::parse("X"),
            z_units: "%".to_string(),
            element_bits: 8,
            values: vec![vec![20.0, 35.0, 50.0, 60.0, 65.0]],
        });

        enrich_document_strategy_meta(&mut xdf);
        xdf
    }
}

fn axis_dimension(axis_node: &roxmltree::Node) -> usize {
    if let Some(dim) = axis_node
        .children()
        .find(|n| n.has_tag_name("dim"))
        .and_then(|n| n.text())
        .and_then(|t| t.parse::<usize>().ok())
    {
        return dim.max(1);
    }
    if let Some(count) = axis_node
        .children()
        .find(|n| n.has_tag_name("indexcount"))
        .and_then(|n| n.text())
        .and_then(|t| t.parse::<usize>().ok())
    {
        return count.max(1);
    }
    1
}

fn embedded_address(node: &roxmltree::Node) -> Option<u32> {
    let data = node
        .children()
        .find(|n| n.has_tag_name("EMBEDDEDDATA"))?;
    parse_hex_address(data.attribute("mmedaddress")).or_else(|| {
        data.children()
            .find(|n| n.has_tag_name("mmedaddress"))
            .and_then(|n| n.text())
            .and_then(|t| parse_hex_address(Some(t)))
    })
}

fn parse_hex_address(text: Option<&str>) -> Option<u32> {
    let t = text?.trim();
    if t.is_empty() {
        return None;
    }
    let s = t.trim_start_matches("0x").trim_start_matches("0X");
    u32::from_str_radix(s, 16).ok()
}

fn labels_look_like_placeholders(labels: &[f64]) -> bool {
    !labels.is_empty()
        && labels
            .iter()
            .enumerate()
            .all(|(i, v)| (*v - i as f64).abs() < 1e-6)
}

fn parse_axis_labels(axis_node: &roxmltree::Node, dim: usize) -> Vec<f64> {
    let mut labels: Vec<Option<f64>> = vec![None; dim];
    for label_node in axis_node.children().filter(|n| n.has_tag_name("LABEL")) {
        let idx = label_node
            .attribute("index")
            .and_then(|t| t.parse::<usize>().ok())
            .unwrap_or(0);
        let text = label_node
            .attribute("value")
            .or_else(|| label_node.text())
            .unwrap_or("0");
        if let Ok(v) = text.trim().parse::<f64>() {
            if idx < dim {
                labels[idx] = Some(v);
            }
        }
    }
    if labels.iter().any(|l| l.is_some()) {
        return labels
            .into_iter()
            .enumerate()
            .map(|(i, v)| v.unwrap_or(i as f64))
            .collect();
    }
    (0..dim).map(|i| i as f64).collect()
}

fn read_axis_labels_from_rom(rom: &[u8], origin: i64, axis: &mut XdfAxis) {
    if axis.address == 0 || axis.dim == 0 {
        return;
    }
    let mut labels = Vec::with_capacity(axis.dim);
    for i in 0..axis.dim {
        let addr = axis.address + i as u32;
        let v = read_scalar_u8(rom, origin, addr, &axis.equation).unwrap_or(i as f64);
        labels.push(v);
    }
    axis.labels = labels;
}

fn read_scalar_u8(rom: &[u8], origin: i64, address: u32, equation: &ScaleEquation) -> Option<f64> {
    let idx = locate(rom, origin, address as i64)?;
    let raw = rom[idx] as f64;
    Some(equation.evaluate(raw))
}

fn read_table_from_rom(rom: &[u8], origin: i64, table: &mut XdfTable) {
    let rows = table.y_axis.dim;
    let cols = table.x_axis.dim;
    if table.values.len() != rows {
        table.values = vec![vec![0.0; cols]; rows];
    }
    for r in 0..rows {
        if table.values[r].len() != cols {
            table.values[r] = vec![0.0; cols];
        }
        for c in 0..cols {
            let offset = (r * cols + c) as u32;
            let addr = table.address + offset;
            if let Some(v) = read_scalar_u8(rom, origin, addr, &table.z_equation) {
                table.values[r][c] = (v * 100.0).round() / 100.0;
            }
        }
    }
}

fn write_table_to_rom(rom: &mut [u8], origin: i64, table: &XdfTable) -> bool {
    let rows = table.values.len();
    if rows == 0 {
        return false;
    }
    let cols = table.values[0].len();
    for r in 0..rows {
        for c in 0..cols {
            if c >= table.values[r].len() {
                continue;
            }
            let offset = (r * cols + c) as u32;
            let addr = table.address + offset;
            if !write_scalar_u8(rom, origin, addr, &table.z_equation, table.values[r][c]) {
                return false;
            }
        }
    }
    true
}

fn write_scalar_u8(
    rom: &mut [u8],
    origin: i64,
    address: u32,
    equation: &ScaleEquation,
    engineering: f64,
) -> bool {
    let Some(idx) = locate(rom, origin, address as i64) else {
        return false;
    };
    let raw = equation.invert(engineering).round().clamp(0.0, 255.0) as u8;
    rom[idx] = raw;
    true
}

#[cfg(test)]
mod rom_axis_tests {
    use super::*;

    #[test]
    fn rom_backed_axes_rebind_after_mock_rom() {
        let xml = std::fs::read_to_string("tests/fixtures/tacoma-bench-table.xdf").expect("fixture");
        let mut doc = XdfDocument::parse(&xml).expect("parse");
        // App startup binds a 0xFF mock ROM first...
        doc.bind_from_rom(&vec![0xFFu8; 0x200], 0);
        assert_eq!(doc.tables[0].x_axis.labels, vec![25500.0, 25500.0]);
        // ...then the user opens a real ROM: axes must follow it.
        let mut rom = vec![0u8; 0x200];
        rom[0x108] = 20;
        rom[0x109] = 40;
        rom[0x110] = 3;
        rom[0x111] = 9;
        doc.bind_from_rom(&rom, 0);
        assert_eq!(doc.tables[0].x_axis.labels, vec![2000.0, 4000.0]);
        assert_eq!(doc.tables[0].y_axis.labels, vec![30.0, 90.0]);
    }
}
