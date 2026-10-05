//! Pure-Rust SMS PDU construction, replacing the `sms-tool_q` helper.
//!
//! `sms-tool_q` is a third-party dependency that has been deliberately
//! discarded from this backend. Everything it used to do — PDU encoding for
//! outgoing messages — is now generated in-process, with support for both
//! encodings plus automatic multipart (concatenated) splitting:
//!
//!   * **GSM 7-bit default alphabet** for messages that fit it (max 160
//!     septets, 153 per part when split), DCS `0x00`.
//!   * **UCS-2** (UTF-16BE) for anything else, chiefly CJK (max 70 chars,
//!     67 per part when split), DCS `0x08`.
//!
//! The SMS-SUBMIT TPDU includes the destination address (TOA 0x91 for
//! international numbers) and, when multiple parts are needed, a 6-byte
//! concatenation Information Element (IEI 0x00) in the user-data header.
//! The service-centre prefix is left empty (`00`) so the modem uses its
//! stored SMSC (configurable via `mt5700m-at sms-set smsc <number>`).

use std::sync::atomic::{AtomicU8, Ordering};

/// A fully encoded SMS-SUBMIT PDU ready to be passed to `AT+CMGS=<len>`.
#[derive(Debug, Clone)]
pub struct SmsPdu {
    /// User-data length for `AT+CMGS=<len>`: number of septets for GSM-7
    /// (including the 6-septet concat header when multipart), number of
    /// octets for UCS-2.
    pub length: u32,
    /// Hex encoding of the TPDU (without any SMSC prefix).
    pub hex: String,
}

impl SmsPdu {
    fn new(length: u32, tpdu: Vec<u8>) -> Self {
        let hex = tpdu.iter().map(|b| format!("{:02X}", b)).collect();
        SmsPdu { length, hex }
    }
}

/// Monotonic reference number for multipart messages (per-process).
static REF: AtomicU8 = AtomicU8::new(1);

fn next_ref() -> u8 {
    let r = REF.fetch_add(1, Ordering::Relaxed);
    r % 255 + 1
}

// ---------------------------------------------------------------- GSM 7-bit

/// GSM 03.38 default 7-bit alphabet (basic set). Characters outside this table
/// force the UCS-2 path — the safe, always-correct fallback.
fn gsm7_encode(ch: char) -> Option<u8> {
    Some(match ch {
        '@' => 0x00, '£' => 0x01, '$' => 0x02, '¥' => 0x03, 'è' => 0x04,
        'é' => 0x05, 'ù' => 0x06, 'ì' => 0x07, 'ò' => 0x08, 'Ç' => 0x09,
        '\n' => 0x0A, 'Ø' => 0x0B, 'ø' => 0x0C, '\r' => 0x0D, 'Å' => 0x0E,
        'å' => 0x0F, 'Δ' => 0x10, '_' => 0x11, 'Φ' => 0x12, 'Γ' => 0x13,
        'Λ' => 0x14, 'Ω' => 0x15, 'Π' => 0x16, 'Ψ' => 0x17, 'Σ' => 0x18,
        'Θ' => 0x19, 'Ξ' => 0x1A, '\u{1b}' => 0x1B, 'Æ' => 0x1C, 'æ' => 0x1D,
        'ß' => 0x1E, 'É' => 0x1F, ' ' => 0x20, '!' => 0x21, '"' => 0x22,
        '#' => 0x23, '¤' => 0x24, '%' => 0x25, '&' => 0x26, '\'' => 0x27,
        '(' => 0x28, ')' => 0x29, '*' => 0x2A, '+' => 0x2B, ',' => 0x2C,
        '-' => 0x2D, '.' => 0x2E, '/' => 0x2F,
        '0' => 0x30, '1' => 0x31, '2' => 0x32, '3' => 0x33, '4' => 0x34,
        '5' => 0x35, '6' => 0x36, '7' => 0x37, '8' => 0x38, '9' => 0x39,
        ':' => 0x3A, ';' => 0x3B, '<' => 0x3C, '=' => 0x3D, '>' => 0x3E,
        '?' => 0x3F, '¡' => 0x40,
        'A' => 0x41, 'B' => 0x42, 'C' => 0x43, 'D' => 0x44, 'E' => 0x45,
        'F' => 0x46, 'G' => 0x47, 'H' => 0x48, 'I' => 0x49, 'J' => 0x4A,
        'K' => 0x4B, 'L' => 0x4C, 'M' => 0x4D, 'N' => 0x4E, 'O' => 0x4F,
        'P' => 0x50, 'Q' => 0x51, 'R' => 0x52, 'S' => 0x53, 'T' => 0x54,
        'U' => 0x55, 'V' => 0x56, 'W' => 0x57, 'X' => 0x58, 'Y' => 0x59,
        'Z' => 0x5A, 'Ä' => 0x5B, 'Ö' => 0x5C, 'Ñ' => 0x5D, 'Ü' => 0x5E,
        '§' => 0x5F, '¿' => 0x60,
        'a' => 0x61, 'b' => 0x62, 'c' => 0x63, 'd' => 0x64, 'e' => 0x65,
        'f' => 0x66, 'g' => 0x67, 'h' => 0x68, 'i' => 0x69, 'j' => 0x6A,
        'k' => 0x6B, 'l' => 0x6C, 'm' => 0x6D, 'n' => 0x6E, 'o' => 0x6F,
        'p' => 0x70, 'q' => 0x71, 'r' => 0x72, 's' => 0x73, 't' => 0x74,
        'u' => 0x75, 'v' => 0x76, 'w' => 0x77, 'x' => 0x78, 'y' => 0x79,
        'z' => 0x7A, 'ä' => 0x7B, 'ö' => 0x7C, 'ñ' => 0x7D, 'ü' => 0x7E,
        'à' => 0x7F,
        _ => return None,
    })
}

/// Pack septets into octets (LSB-first). Length in septets.
fn pack_septets(septets: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity((septets.len() * 7 + 7) / 8);
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    for &s in septets {
        acc |= (s as u32) << bits;
        bits += 7;
        while bits >= 8 {
            out.push((acc & 0xFF) as u8);
            acc >>= 8;
            bits -= 8;
        }
    }
    if bits > 0 {
        out.push((acc & 0xFF) as u8);
    }
    out
}

/// Encode `text` as GSM-7 septets, or `None` if it contains chars outside the
/// default alphabet.
fn gsm7_septets(text: &str) -> Option<Vec<u8>> {
    let mut v = Vec::with_capacity(text.chars().count());
    for ch in text.chars() {
        if ch == '\r' || ch == '\n' {
            v.push(0x0A); // default-alphabet newline
            continue;
        }
        v.push(gsm7_encode(ch)?);
    }
    Some(v)
}

/// UTF-16BE octets (without BOM).
fn ucs2_be(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() * 2);
    for unit in text.encode_utf16() {
        out.extend_from_slice(&unit.to_be_bytes());
    }
    out
}

// ---------------------------------------------------------------- Address

/// Build the TP-DA segment: address-length (nibbles), TOA, swapped digits.
fn encode_destination(number: &str) -> Vec<u8> {
    let intl = number.starts_with('+');
    let digits: String = number.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return vec![0x00, 0x91, 0xF0];
    }
    let nibbles: u8 = if digits.len() % 2 == 0 {
        digits.len() as u8
    } else {
        (digits.len() + 1) as u8
    };
    let toa: u8 = if intl { 0x91 } else { 0x81 };
    let max_pairs = (usize::from(nibbles)) / 2;
    let chars: Vec<char> = digits.chars().collect();
    let mut body = Vec::with_capacity(max_pairs + 1);
    for i in 0..max_pairs {
        let hi = chars[i * 2].to_digit(16).unwrap_or(0) as u8;
        let lo = chars
            .get(i * 2 + 1)
            .and_then(|c| c.to_digit(16))
            .unwrap_or(15) as u8;
        body.push((lo << 4) | hi); // nibbles swapped, low nibble first
    }
    let mut out = Vec::with_capacity(2 + body.len());
    out.push(nibbles);
    out.push(toa);
    out.extend_from_slice(&body);
    out
}

// ---------------------------------------------------------------- PDU assembly

/// SMS-SUBMIT first octet. UDHI set when the user data carries the concat
/// header; VPF=00 (no validity period), RP off, MTI=01.
const fn first_octet(udhi: bool) -> u8 {
    if udhi { 0x41 } else { 0x01 }
}

fn assemble_tpdu(number: &str, dcs: u8, length: u32, udhi: bool, data: &[u8]) -> Vec<u8> {
    let mut tpdu = Vec::new();
    tpdu.push(first_octet(udhi));
    tpdu.push(0x00); // TP-MR
    tpdu.extend_from_slice(&encode_destination(number));
    tpdu.push(0x00); // TP-PID
    tpdu.push(dcs);
    tpdu.push(length as u8);
    tpdu.extend_from_slice(data);
    tpdu
}

/// Build the final list of PDUs for `number`/`text`. Returns at least one PDU;
/// long text is split into concatenated parts carrying a UDHI header.
pub fn encode(number: &str, text: &str) -> Vec<SmsPdu> {
    let refnum = next_ref();

    if let Some(septets) = gsm7_septets(text) {
        const SINGLE_MAX: usize = 160;
        const PART_MAX: usize = 153; // 160 - 7 (6-byte header + pad bit)
        if septets.len() <= SINGLE_MAX {
            let packed = pack_septets(&septets);
            let tpdu = assemble_tpdu(number, 0x00, septets.len() as u32, false, &packed);
            return vec![SmsPdu::new(septets.len() as u32, tpdu)];
        }
        let total_parts = (septets.len() + PART_MAX - 1) / PART_MAX;
        let mut pdus = Vec::with_capacity(total_parts);
        for seq in 0..total_parts {
            let start = seq * PART_MAX;
            let end = (start + PART_MAX).min(septets.len());
            // Concatenation header as septets: IEI, IEL, ref, total, seq.
            let mut part: Vec<u8> = vec![0x00, 0x03, refnum, total_parts as u8, seq as u8 + 1];
            part.extend_from_slice(&septets[start..end]);
            let packed = pack_septets(&part);
            let tpdu = assemble_tpdu(number, 0x00, part.len() as u32, true, &packed);
            pdus.push(SmsPdu::new(part.len() as u32, tpdu));
        }
        return pdus;
    }

    // UCS-2 path.
    let octets = ucs2_be(text);
    const SINGLE_MAX: usize = 140; // 70 UCS-2 chars
    const PART_MAX: usize = 134; // 140 - 6 header octets
    if octets.len() <= SINGLE_MAX {
        let tpdu = assemble_tpdu(number, 0x08, octets.len() as u32, false, &octets);
        return vec![SmsPdu::new(octets.len() as u32, tpdu)];
    }
    let total_parts = (octets.len() + PART_MAX - 1) / PART_MAX;
    let mut pdus = Vec::with_capacity(total_parts);
    for seq in 0..total_parts {
        let start = seq * PART_MAX;
        let end = (start + PART_MAX).min(octets.len());
        let mut data: Vec<u8> = vec![0x00, 0x03, refnum, total_parts as u8, seq as u8 + 1];
        data.extend_from_slice(&octets[start..end]);
        let tpdu = assemble_tpdu(number, 0x08, data.len() as u32, true, &data);
        pdus.push(SmsPdu::new(data.len() as u32, tpdu));
    }
    pdus
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gsm7_alphabet_boundaries() {
        assert_eq!(gsm7_encode('A'), Some(0x41));
        assert_eq!(gsm7_encode('€'), None); // extension set -> UCS2 fallback
        assert_eq!(gsm7_encode('中'), None); // CJK -> UCS2
    }

    #[test]
    fn pack_septets_known_vector() {
        // "hello" -> E8 32 9B FD 06 (5 septets packed into 5 octets,
        // LSB-first, the canonical GSM 03.38 example).
        let septets = [0x68u8, 0x65, 0x6C, 0x6C, 0x6F];
        let packed = pack_septets(&septets);
        let hd = packed.iter().map(|b| format!("{:02X}", b)).collect::<String>();
        assert_eq!(hd, "E8329BFD06");
    }

    #[test]
    fn single_ascii_pdu_shapes() {
        let pdus = encode("+8613800138000", "hello");
        assert_eq!(pdus.len(), 1);
        assert!(pdus[0].hex.starts_with("01")); // SMS-SUBMIT, no UDHI
        // 14 digits -> address-length nibble 0x0E, TOA 0x91 (intl).
        assert!(pdus[0].hex.contains("0E91"));
        assert_eq!(pdus[0].length, 5); // 5 septets
    }

    #[test]
    fn ucs2_pdu_for_chinese() {
        let pdus = encode("+8613800138000", "你好");
        assert_eq!(pdus.len(), 1);
        assert!(pdus[0].hex.contains("08")); // DCS UCS-2
        assert_eq!(pdus[0].length, 4); // 4 octets
        // UDL byte 0x04 followed by the 4 UTF-16BE data octets.
        assert!(pdus[0].hex.ends_with("044F60597D"));
        assert!(pdus[0].hex.contains("4F60597D"));
    }

    #[test]
    fn multipart_split_adds_udhi() {
        let text = "x".repeat(200);
        let pdus = encode("13800138000", &text);
        assert!(pdus.len() > 1, "expected >1 part, got {}", pdus.len());
        for p in &pdus {
            assert!(p.hex.starts_with("41")); // SMS-SUBMIT + UDHI
        }
    }

    #[test]
    fn multipart_ucs2_split() {
        let text = "中".repeat(90); // 180 octets -> split UCS-2
        let pdus = encode("13800138000", &text);
        assert!(pdus.len() > 1);
        assert!(pdus[0].hex.starts_with("41"));
        assert!(pdus[0].hex.contains("08"));
    }

    #[test]
    fn destination_swaps_nibbles() {
        // "+8613800138000": 14 digits -> 14 nibbles, TOA 91.
        let d = encode_destination("+8613800138000");
        assert_eq!(d[0], 14);
        assert_eq!(d[1], 0x91);
        assert_eq!(d[2], 0x68); // '8','6' swapped
        assert_eq!(d[3], 0x31); // '1','3' swapped
    }

    #[test]
    fn odd_length_number_pads_f() {
        // 3 digits -> 4 nibbles, the trailing odd digit padded with F.
        let d = encode_destination("+861");
        assert_eq!(d[0], 4);
        assert_eq!(d[1], 0x91);
        // digits "861" -> pairs: ('8','6')='8'->0x68, then '1' with F pad.
        assert_eq!(d[2], 0x68);
        assert_eq!(d[3], 0xF1);
    }
}