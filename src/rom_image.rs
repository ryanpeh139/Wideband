//! ROM image loaders: raw `.bin` and Toyota Motorola S-record `.cuw` payloads.
//! Parity with C# `CalibrationFiles` / `CuwFile` (fail-closed on encrypted/opaque bodies).

pub const MAX_CUW_SPAN: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RomImage {
    pub origin: u64,
    pub bytes: Vec<u8>,
    pub note: String,
}

impl RomImage {
    /// Load from file bytes; extension hint is `"bin"` or `"cuw"` (case-insensitive).
    pub fn from_bytes(extension: &str, data: &[u8]) -> Result<Self, String> {
        if data.len() >= 8 && data[..8] == *b"VFTBLPE1" {
            return Err("Encrypted or opaque profile bodies are not opened.".to_string());
        }
        let ext = extension.to_lowercase();
        if ext == "cuw" {
            CuwFile::parse(data).map(|c| RomImage {
                origin: c.origin as u64,
                bytes: c.image,
                note: c.note,
            })
        } else {
            Ok(RomImage {
                origin: 0,
                bytes: data.to_vec(),
                note: String::new(),
            })
        }
    }

    /// Resize `buffer` and copy this image at `origin` (fills gaps with `0xFF`).
    pub fn apply_to_buffer(&self, buffer: &mut Vec<u8>) {
        let end = self.origin as usize + self.bytes.len();
        if buffer.len() < end {
            buffer.resize(end, 0xFF);
        }
        let start = self.origin as usize;
        if start + self.bytes.len() <= buffer.len() {
            buffer[start..start + self.bytes.len()].copy_from_slice(&self.bytes);
        }
    }

    /// Replace buffer entirely when origin is zero; otherwise merge like a partial CUW load.
    pub fn load_into_buffer(&self, buffer: &mut Vec<u8>, replace_when_origin_zero: bool) {
        if self.origin == 0 && replace_when_origin_zero {
            *buffer = self.bytes.clone();
        } else {
            self.apply_to_buffer(buffer);
        }
    }
}

#[derive(Clone, Debug)]
pub struct CuwFile {
    pub origin: u32,
    pub image: Vec<u8>,
    pub note: String,
}

impl CuwFile {
    pub fn parse(content: &[u8]) -> Result<Self, String> {
        if content.is_empty() {
            return Err("The CUW file has no calibration payload.".to_string());
        }
        let start = content.iter().position(|&b| b != 0).unwrap_or(content.len());
        if start >= content.len() {
            return Err("The CUW file has no calibration payload.".to_string());
        }
        let body = &content[start..];
        let text = String::from_utf8_lossy(body);
        let lines: Vec<&str> = text.lines().map(|l| l.trim_end_matches('\r')).collect();
        let first_s = lines.iter().position(|l| is_s_record_shape(l.trim()));
        if let Some(first) = first_s {
            return parse_s_records(&lines, first);
        }
        parse_remainder(body)
    }
}

fn parse_remainder(body: &[u8]) -> Result<CuwFile, String> {
    let mut index = 0usize;
    while index < body.len() && is_header_byte(body[index]) {
        index += 1;
    }
    if index >= body.len() || body.len() - index < 16 {
        return Err("The CUW file has no calibration payload.".to_string());
    }
    let payload = body[index..].to_vec();
    let note = clean_note(String::from_utf8_lossy(&body[..index]).as_ref());
    Ok(CuwFile {
        origin: 0,
        image: payload,
        note,
    })
}

fn parse_s_records(lines: &[&str], first: usize) -> Result<CuwFile, String> {
    let mut header = String::new();
    for i in 0..first {
        if !header.is_empty() {
            header.push('\n');
        }
        header.push_str(lines[i]);
    }
    let mut chunks: Vec<(u32, Vec<u8>)> = Vec::new();
    let mut declared: Option<i32> = None;
    let mut data_records = 0i32;

    for (line_idx, line) in lines.iter().enumerate().skip(first) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !is_s_record_shape(trimmed) {
            return Err(format!(
                "The CUW payload has an invalid S-record on line {}.",
                line_idx + 1
            ));
        }
        let record = parse_s_line(trimmed, line_idx + 1)?;
        match record.record_type {
            '1' | '2' | '3' => {
                data_records += 1;
                if !record.data.is_empty() {
                    chunks.push((record.address, record.data));
                }
            }
            '5' => {
                declared = Some(record.address as i32);
            }
            _ => {}
        }
    }

    if let Some(count) = declared {
        if count != data_records {
            return Err("The CUW S5 count does not match the data records.".to_string());
        }
    }

    if chunks.is_empty() {
        return Err("The CUW file has no calibration payload.".to_string());
    }

    let mut min_addr = chunks[0].0;
    let mut max_end = chunks[0].0 as u64 + chunks[0].1.len() as u64;
    for (addr, data) in &chunks[1..] {
        min_addr = min_addr.min(*addr);
        let end = *addr as u64 + data.len() as u64;
        if end > max_end {
            max_end = end;
        }
    }

    let span = (max_end - min_addr as u64) as usize;
    if span == 0 || span > MAX_CUW_SPAN {
        return Err("The CUW payload is larger than 8 MB.".to_string());
    }

    let mut image = vec![0xFFu8; span];
    for (addr, data) in chunks {
        let offset = (addr - min_addr) as usize;
        if offset + data.len() <= image.len() {
            image[offset..offset + data.len()].copy_from_slice(&data);
        }
    }

    Ok(CuwFile {
        origin: min_addr,
        image,
        note: clean_note(&header),
    })
}

struct ParsedSRecord {
    record_type: char,
    address: u32,
    data: Vec<u8>,
}

fn parse_s_line(line: &str, number: usize) -> Result<ParsedSRecord, String> {
    if line.len() < 4 {
        return Err(format!("Invalid S-record on line {}.", number));
    }
    let rec_type = line.chars().nth(1).unwrap().to_ascii_uppercase();
    let addr_len = address_bytes(rec_type);
    if addr_len == 0 {
        return Err(format!("Invalid S-record on line {}.", number));
    }

    let hex_payload = &line[2..];
    if hex_payload.len() < 4 || (hex_payload.len() & 1) != 0 {
        return Err(format!("Invalid S-record on line {}.", number));
    }

    let mut bytes = Vec::with_capacity(hex_payload.len() / 2);
    for i in (0..hex_payload.len()).step_by(2) {
        let b = u8::from_str_radix(&hex_payload[i..i + 2], 16)
            .map_err(|_| format!("Invalid hex on line {}.", number))?;
        bytes.push(b);
    }

    let count = bytes[0] as usize;
    if bytes.len() != count + 1 {
        return Err(format!("Invalid S-record length on line {}.", number));
    }

    let mut sum = 0u8;
    for &b in &bytes {
        sum = sum.wrapping_add(b);
    }
    if sum != 0xFF {
        return Err(format!("S-record checksum failed on line {}.", number));
    }

    if (bytes[0] as usize) < addr_len + 1 {
        return Err(format!("Invalid S-record on line {}.", number));
    }

    let mut addr = 0u32;
    for i in 0..addr_len {
        addr = (addr << 8) | (bytes[1 + i] as u32);
    }
    let data_len = bytes[0] as usize - addr_len - 1;
    let data = bytes[1 + addr_len..1 + addr_len + data_len].to_vec();

    Ok(ParsedSRecord {
        record_type: rec_type,
        address: addr,
        data,
    })
}

fn address_bytes(rec_type: char) -> usize {
    match rec_type {
        '0' | '1' | '5' | '9' => 2,
        '2' | '8' => 3,
        '3' | '7' => 4,
        _ => 0,
    }
}

fn is_s_record_shape(line: &str) -> bool {
    line.len() >= 4 && (line.starts_with('S') || line.starts_with('s')) && line[2..4].chars().all(|c| c.is_ascii_hexdigit())
}

fn is_header_byte(b: u8) -> bool {
    b == 0x0D || b == 0x0A || b == 0x09 || b == 0x20 || (b >= 0x20 && b < 0x7F)
}

fn clean_note(text: &str) -> String {
    text.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_vf_encrypted_header() {
        let data = b"VFTBLPE1xxxx";
        let err = RomImage::from_bytes("bin", data).unwrap_err();
        assert!(err.contains("opaque"));
    }

    #[test]
    fn cuw_s3_merge_span() {
        let s_text = "HDR TOYOTA CUW TEST\nS307000100001122C4\nS70500010000F9\n";
        let img = RomImage::from_bytes("cuw", s_text.as_bytes()).expect("parse");
        assert_eq!(img.origin, 0x10000);
        assert_eq!(img.bytes.len(), 2);
        assert_eq!(img.bytes[0], 0x11);
        assert_eq!(img.bytes[1], 0x22);
    }

    #[test]
    fn bin_load_roundtrip() {
        let raw = vec![0xDE, 0xAD, 0xBE, 0xEF];
        let img = RomImage::from_bytes("bin", &raw).expect("bin");
        assert_eq!(img.origin, 0);
        assert_eq!(img.bytes, raw);
    }

    #[test]
    fn apply_to_buffer_extends() {
        let img = RomImage {
            origin: 4,
            bytes: vec![0xAA, 0xBB],
            note: String::new(),
        };
        let mut buf = vec![0u8; 3];
        img.apply_to_buffer(&mut buf);
        assert_eq!(buf.len(), 6);
        assert_eq!(buf[4], 0xAA);
        assert_eq!(buf[5], 0xBB);
    }
}
