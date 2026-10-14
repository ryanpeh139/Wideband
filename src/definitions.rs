use roxmltree::Document;

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
// Motorola S-Record (.cuw) File Extractor
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct CuwFile {
    pub origin: u32,
    pub image: Vec<u8>,
    pub note: String,
}

impl CuwFile {
    pub fn parse(content: &[u8]) -> Result<Self, String> {
        let text = String::from_utf8_lossy(content);
        let lines = text.lines().peekable();

        let mut note = String::new();
        let mut s_records = Vec::new();

        for line in lines {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            if trimmed.starts_with('S') || trimmed.starts_with('s') {
                s_records.push(trimmed.to_string());
            } else if s_records.is_empty() {
                if !note.is_empty() {
                    note.push('\n');
                }
                note.push_str(trimmed);
            }
        }

        if s_records.is_empty() {
            // Raw binary fallback
            return Ok(Self {
                origin: 0,
                image: content.to_vec(),
                note: "Raw binary ROM".to_string(),
            });
        }

        let mut chunks: Vec<(u32, Vec<u8>)> = Vec::new();
        let mut min_addr = u32::MAX;
        let mut max_addr = 0u32;

        for (line_idx, line) in s_records.iter().enumerate() {
            if line.len() < 4 {
                continue;
            }
            let rec_type = line.chars().nth(1).unwrap();
            let addr_len = match rec_type {
                '1' => 2,
                '2' => 3,
                '3' => 4,
                _ => continue, // S0 header or S7/S8/S9 termination
            };

            let count_hex = &line[2..4];
            let count = u8::from_str_radix(count_hex, 16)
                .map_err(|e| format!("Invalid S-record count at line {}: {}", line_idx, e))? as usize;

            if line.len() < 4 + count * 2 {
                continue;
            }

            let hex_payload = &line[4..4 + count * 2];
            let mut bytes = Vec::new();
            for i in (0..hex_payload.len()).step_by(2) {
                if i + 2 <= hex_payload.len() {
                    let b = u8::from_str_radix(&hex_payload[i..i + 2], 16)
                        .map_err(|e| format!("Invalid hex byte at line {}: {}", line_idx, e))?;
                    bytes.push(b);
                }
            }

            if bytes.len() <= addr_len + 1 {
                continue;
            }

            // Checksum verification: sum of count + bytes must be 0xFF
            let mut sum = count as u8;
            for &b in &bytes {
                sum = sum.wrapping_add(b);
            }
            if sum != 0xFF {
                return Err(format!("S-record checksum mismatch at line {}", line_idx));
            }

            // Parse address
            let mut addr = 0u32;
            for i in 0..addr_len {
                addr = (addr << 8) | (bytes[i] as u32);
            }

            let data = bytes[addr_len..bytes.len() - 1].to_vec();
            if !data.is_empty() {
                min_addr = min_addr.min(addr);
                max_addr = max_addr.max(addr + data.len() as u32);
                chunks.push((addr, data));
            }
        }

        if chunks.is_empty() {
            return Err("No S1/S2/S3 data records found in CUW file".to_string());
        }

        let total_size = (max_addr - min_addr) as usize;
        let mut image = vec![0xFFu8; total_size];

        for (addr, data) in chunks {
            let offset = (addr - min_addr) as usize;
            if offset + data.len() <= total_size {
                image[offset..offset + data.len()].copy_from_slice(&data);
            }
        }

        Ok(Self {
            origin: min_addr,
            image,
            note,
        })
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
}

#[derive(Clone, Debug)]
pub struct XdfTable {
    pub id: String,
    pub title: String,
    pub description: String,
    pub address: u32,
    pub x_axis: XdfAxis,
    pub y_axis: XdfAxis,
    pub z_equation: ScaleEquation,
    pub z_units: String,
    pub values: Vec<Vec<f64>>,
}

#[derive(Clone, Debug)]
pub struct XdfConstant {
    pub id: String,
    pub title: String,
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

        Ok(xdf)
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
        };
        let mut y_axis = XdfAxis {
            id: 'y',
            dim: 12,
            address: 0,
            units: "MAP (kPa)".to_string(),
            equation: ScaleEquation::parse("X"),
            labels: Vec::new(),
        };
        let mut z_equation = ScaleEquation::parse("X");
        let mut z_units = "VE %".to_string();
        let mut table_addr = 0u32;

        for axis_node in node.children().filter(|n| n.has_tag_name("XDFAXIS")) {
            let axis_id = axis_node.attribute("id").unwrap_or("x");
            let dim = axis_node.children().find(|n| n.has_tag_name("dim"))
                .and_then(|n| n.text()).and_then(|t| t.parse().ok()).unwrap_or(1);
            let units = axis_node.children().find(|n| n.has_tag_name("units"))
                .and_then(|n| n.text()).unwrap_or("").to_string();
            let eq_str = axis_node.children().find(|n| n.has_tag_name("MATH"))
                .and_then(|n| n.attribute("equation")).unwrap_or("X");

            let addr = axis_node.children().find(|n| n.has_tag_name("EMBEDDEDDATA"))
                .and_then(|n| n.attribute("mmedaddress"))
                .and_then(|a| u32::from_str_radix(a.trim_start_matches("0x"), 16).ok())
                .unwrap_or(0);

            let axis = XdfAxis {
                id: axis_id.chars().next().unwrap_or('x'),
                dim,
                address: addr,
                units,
                equation: ScaleEquation::parse(eq_str),
                labels: (0..dim).map(|i| (i * 500) as f64).collect(),
            };

            match axis_id {
                "x" | "X" => x_axis = axis,
                "y" | "Y" => y_axis = axis,
                "z" | "Z" => {
                    table_addr = addr;
                    z_equation = ScaleEquation::parse(eq_str);
                    z_units = axis.units;
                }
                _ => {}
            }
        }

        // Generate synthetic grid values for editor display
        let mut values = vec![vec![0.0; x_axis.dim]; y_axis.dim];
        for row in 0..y_axis.dim {
            for col in 0..x_axis.dim {
                values[row][col] = 45.0 + (col as f64 * 3.2) + (row as f64 * 2.5);
            }
        }

        Some(XdfTable {
            id,
            title,
            description,
            address: table_addr,
            x_axis,
            y_axis,
            z_equation,
            z_units,
            values,
        })
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

        Some(XdfConstant {
            id,
            title,
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

        Some(XdfFlag {
            id,
            title,
            description,
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
            description: "Primary Speed-Density VE fuel delivery table vs RPM and MAP kPa.".to_string(),
            address: 0x10000,
            x_axis: XdfAxis {
                id: 'x',
                dim: x_rpm.len(),
                address: 0x10100,
                units: "RPM".to_string(),
                equation: ScaleEquation::parse("X"),
                labels: x_rpm,
            },
            y_axis: XdfAxis {
                id: 'y',
                dim: y_map.len(),
                address: 0x10180,
                units: "MAP (kPa)".to_string(),
                equation: ScaleEquation::parse("X"),
                labels: y_map,
            },
            z_equation: ScaleEquation::parse("X * 0.1"),
            z_units: "%".to_string(),
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
            description: "Target ignition advance in degrees BTDC.".to_string(),
            address: 0x10800,
            x_axis: XdfAxis {
                id: 'x',
                dim: spark_rpm.len(),
                address: 0x10880,
                units: "RPM".to_string(),
                equation: ScaleEquation::parse("X"),
                labels: spark_rpm,
            },
            y_axis: XdfAxis {
                id: 'y',
                dim: spark_load.len(),
                address: 0x108C0,
                units: "Load %".to_string(),
                equation: ScaleEquation::parse("X"),
                labels: spark_load,
            },
            z_equation: ScaleEquation::parse("X * 0.25"),
            z_units: "deg BTDC".to_string(),
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
            description: "Target equivalence ratio (Lambda). 1.0 = Stoichiometric.".to_string(),
            address: 0x11000,
            x_axis: XdfAxis {
                id: 'x',
                dim: lambda_rpm.len(),
                address: 0x11050,
                units: "RPM".to_string(),
                equation: ScaleEquation::parse("X"),
                labels: lambda_rpm,
            },
            y_axis: XdfAxis {
                id: 'y',
                dim: lambda_map.len(),
                address: 0x11080,
                units: "MAP (kPa)".to_string(),
                equation: ScaleEquation::parse("X"),
                labels: lambda_map,
            },
            z_equation: ScaleEquation::parse("X * 0.0078125"),
            z_units: "Lambda".to_string(),
            values: lambda_values,
        });

        // Constants
        xdf.constants.push(XdfConstant {
            id: "rev_limit".to_string(),
            title: "Hard Rev Limiter".to_string(),
            address: 0x12000,
            units: "RPM".to_string(),
            equation: ScaleEquation::parse("X"),
            value: 6500.0,
        });
        xdf.constants.push(XdfConstant {
            id: "speed_limiter".to_string(),
            title: "Vehicle Top Speed Limiter".to_string(),
            address: 0x12004,
            units: "MPH".to_string(),
            equation: ScaleEquation::parse("X"),
            value: 115.0,
        });

        // Flags
        xdf.flags.push(XdfFlag {
            id: "flag_vvt_enable".to_string(),
            title: "Variable Valve Timing (VVT-i) Active".to_string(),
            description: "Enables intake and exhaust camshaft phaser closed loop positioning.".to_string(),
            address: 0x13000,
            mask: 0x01,
            is_set: true,
        });
        xdf.flags.push(XdfFlag {
            id: "flag_cat_overheat".to_string(),
            title: "Catalytic Converter Overtemp Protection (COT)".to_string(),
            description: "Enables fuel enrichment dumping when catalytic bed temp model exceeds 890 C.".to_string(),
            address: 0x13000,
            mask: 0x02,
            is_set: false,
        });
        xdf.flags.push(XdfFlag {
            id: "flag_launch_control".to_string(),
            title: "Motorsport Launch Control (2-Step)".to_string(),
            description: "Enables clutch/brake switch stationary RPM limiting and turbo spool strategy.".to_string(),
            address: 0x13000,
            mask: 0x04,
            is_set: true,
        });

        xdf
    }
}
