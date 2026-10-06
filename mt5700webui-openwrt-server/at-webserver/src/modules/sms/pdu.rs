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

/// Payload ceilings from 3GPP 23.040: what fits in one SMS and what fits in one
/// part of a concatenated SMS (the part budget is the single budget minus the
/// 6-octet UDH, expressed in septets for GSM-7 and octets for UCS-2).
///
/// One copy for the encoder and the compose-hint statistics, so the hint can
/// never disagree with what `encode` actually sends.
const GSM7_SINGLE_MAX: usize = 160;
const GSM7_PART_MAX: usize = 153; // 160 - 7 (6-byte header + pad bit)
const UCS2_SINGLE_MAX: usize = 140; // 70 UCS-2 characters
const UCS2_PART_MAX: usize = 134; // 140 - 6 header octets

/// Parts a payload of `units` needs: one while it fits, then ceil(units/part).
fn part_count(units: usize, single_max: usize, part_max: usize) -> usize {
    if units <= single_max {
        1
    } else {
        (units + part_max - 1) / part_max
    }
}

/// How a message will be sent — what the compose hint next to the input box
/// shows. Pure arithmetic on the same tables the encoder uses: no AT traffic.
#[derive(Debug, Clone, PartialEq)]
pub struct MessageStats {
    /// `"7bit"` (GSM default alphabet) or `"UCS2"`.
    pub encoding: &'static str,
    /// Characters in the message (the hint's "N 字").
    pub chars: usize,
    /// Parts the encoder will produce; 0 for empty text.
    pub parts: usize,
}

/// Statistics for `text` without encoding it (a send re-encodes for real).
pub fn message_stats(text: &str) -> MessageStats {
    let chars = text.chars().count();
    if text.is_empty() {
        return MessageStats {
            encoding: "7bit",
            chars: 0,
            parts: 0,
        };
    }
    match gsm7_septets(text) {
        Some(septets) => MessageStats {
            encoding: "7bit",
            chars,
            parts: part_count(septets.len(), GSM7_SINGLE_MAX, GSM7_PART_MAX),
        },
        None => MessageStats {
            encoding: "UCS2",
            chars,
            parts: part_count(ucs2_be(text).len(), UCS2_SINGLE_MAX, UCS2_PART_MAX),
        },
    }
}

/// Build the final list of PDUs for `number`/`text`. Returns at least one PDU;
/// long text is split into concatenated parts carrying a UDHI header.
pub fn encode(number: &str, text: &str) -> Vec<SmsPdu> {
    let refnum = next_ref();

    if let Some(septets) = gsm7_septets(text) {
        if septets.len() <= GSM7_SINGLE_MAX {
            let packed = pack_septets(&septets);
            let tpdu = assemble_tpdu(number, 0x00, septets.len() as u32, false, &packed);
            return vec![SmsPdu::new(septets.len() as u32, tpdu)];
        }
        let total_parts = part_count(septets.len(), GSM7_SINGLE_MAX, GSM7_PART_MAX);
        let mut pdus = Vec::with_capacity(total_parts);
        for seq in 0..total_parts {
            let start = seq * GSM7_PART_MAX;
            let end = (start + GSM7_PART_MAX).min(septets.len());
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
    if octets.len() <= UCS2_SINGLE_MAX {
        let tpdu = assemble_tpdu(number, 0x08, octets.len() as u32, false, &octets);
        return vec![SmsPdu::new(octets.len() as u32, tpdu)];
    }
    let total_parts = part_count(octets.len(), UCS2_SINGLE_MAX, UCS2_PART_MAX);
    let mut pdus = Vec::with_capacity(total_parts);
    for seq in 0..total_parts {
        let start = seq * UCS2_PART_MAX;
        let end = (start + UCS2_PART_MAX).min(octets.len());
        let mut data: Vec<u8> = vec![0x00, 0x03, refnum, total_parts as u8, seq as u8 + 1];
        data.extend_from_slice(&octets[start..end]);
        let tpdu = assemble_tpdu(number, 0x08, data.len() as u32, true, &data);
        pdus.push(SmsPdu::new(data.len() as u32, tpdu));
    }
    pdus
}

// ------------------------------------------------------------- GSM 7-bit decode

/// GSM 03.38 default alphabet, indexed by septet value. The encoder above
/// writes exactly this alphabet (including `\u{1b}` for the escape), so a
/// decoded message always round-trips through `encode`.
pub const GSM7_ALPHABET: [char; 128] = [
    '@', '£', '$', '¥', 'è', 'é', 'ù', 'ì',
    'ò', 'Ç', '\n', 'Ø', 'ø', '\r', 'Å', 'å',
    'Δ', '_', 'Φ', 'Γ', 'Λ', 'Ω', 'Π', 'Ψ',
    'Σ', 'Θ', 'Ξ', '\u{1b}', 'Æ', 'æ', 'ß', 'É',
    ' ', '!', '"', '#', '¤', '%', '&', '\'',
    '(', ')', '*', '+', ',', '-', '.', '/',
    '0', '1', '2', '3', '4', '5', '6', '7',
    '8', '9', ':', ';', '<', '=', '>', '?',
    '¡', 'A', 'B', 'C', 'D', 'E', 'F', 'G',
    'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O',
    'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W',
    'X', 'Y', 'Z', 'Ä', 'Ö', 'Ñ', 'Ü', '§',
    '¿', 'a', 'b', 'c', 'd', 'e', 'f', 'g',
    'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o',
    'p', 'q', 'r', 's', 't', 'u', 'v', 'w',
    'x', 'y', 'z', 'ä', 'ö', 'ñ', 'ü', 'à',
];

/// Characters reachable through the 0x1B escape sequence.
fn gsm7_escaped(code: u8) -> Option<char> {
    Some(match code {
        0x0A => '\u{0c}', // form feed
        0x14 => '^',
        0x28 => '{',
        0x29 => '}',
        0x2F => '\\',
        0x3C => '[',
        0x3D => '~',
        0x3E => ']',
        0x40 => '|',
        0x65 => '\u{20ac}', // €
        _ => return None,
    })
}

/// One septet -> the character it means (handling the escape sequence).
fn decode_septets(septets: &[u8], escape_state: &mut bool) -> String {
    let mut out = String::new();
    for &s in septets {
        if *escape_state {
            *escape_state = false;
            if let Some(c) = gsm7_escaped(s) {
                out.push(c);
                continue;
            }
            // Unknown escape: keep the escaped character itself.
        }
        if s == 0x1B {
            *escape_state = true;
            continue;
        }
        out.push(GSM7_ALPHABET[(s & 0x7F) as usize]);
    }
    out
}

/// Unpack `count` septets from GSM 7-bit packed octets.
fn unpack_septets(octets: &[u8], count: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(count);
    let mut bit = 0usize;
    for _ in 0..count {
        let byte = bit / 8;
        let shift = bit % 8;
        // Zeros past the end are padding, not data.
        let lo = octets.get(byte).copied().unwrap_or(0) as u16;
        let hi = octets.get(byte + 1).copied().unwrap_or(0) as u16;
        let value = ((lo >> shift) | (hi << (8 - shift))) & 0x7F;
        out.push(value as u8);
        bit += 7;
    }
    out
}

// ------------------------------------------------------------ SMS-DELIVER

/// A decoded incoming message.
#[derive(Debug, Clone, PartialEq)]
pub struct DeliverSms {
    pub number: String,
    pub text: String,
    /// Service-centre timestamp, formatted `YY/MM/DD,HH:MM:SS` — the shape the
    /// UI's time parser and formatter expect.
    pub time: String,
    /// Concatenation reference/total/sequence for multipart messages.
    pub concat: Option<(u8, u8, u8)>,
}

fn hex_byte(hex: &[u8], i: usize) -> Option<u8> {
    let hi = (hex.get(i * 2).copied() as char).to_digit(16)?;
    let lo = (hex.get(i * 2 + 1).copied() as char).to_digit(16)?;
    Some((hi * 16 + lo) as u8)
}

/// Decode a semi-octet (BCD) address: digits, `*`/`#`/`a`-`f` for the special
/// nibbles, `F` marks the unused half of the last octet.
fn decode_address(hex: &[u8], start: usize, digits: usize) -> String {
    let mut out = String::new();
    for i in 0..digits {
        let Some(byte) = hex_byte(hex, start + i / 2) else {
            break;
        };
        let nibble = if i % 2 == 0 { byte & 0x0F } else { byte >> 4 };
        match nibble {
            0x0..=0x9 => out.push((b'0' + nibble) as char),
            0x0A => out.push('*'),
            0x0B => out.push('#'),
            0x0C => out.push('a'),
            0x0D => out.push('b'),
            0x0E => out.push('c'),
            0x0F => {}
            _ => {}
        }
    }
    out
}

fn decode_digits(hex: &[u8], start: usize, count: usize) -> Option<u8> {
    let mut value = 0u32;
    for i in 0..count {
        let d = (hex.get(start + i).copied() as char).to_digit(16)?;
        value = value * 16 + d;
    }
    Some(value as u8)
}

/// Decode one SMS-DELIVER PDU (the hex `AT+CMGL` returns, without the leading
/// SMSC field, or with it — both are handled).
pub fn decode_deliver(hex_str: &str) -> Option<DeliverSms> {
    let hex: Vec<u8> = hex_str
        .trim()
        .bytes()
        .filter(|b| !b.is_ascii_whitespace())
        .collect();
    if hex.len() < 20 || hex.len() % 2 != 0 {
        return None;
    }

    // SMSC prefix: a length octet followed by that many bytes (often `00`).
    let smsc_len = decode_digits(&hex, 0, 2)? as usize;
    let mut i = 1 + smsc_len;
    let first_octet = decode_digits(&hex, i, 2)?;
    i += 1;
    // TP-MTI must be 00 (SMS-DELIVER); 01 is a SUBMIT we cannot render.
    if first_octet & 0x03 != 0 {
        return None;
    }
    let udhi = first_octet & 0x40 != 0;

    // TP-OA: length (digits), type-of-address, then the packed digits.
    let oa_digits = decode_digits(&hex, i, 2)? as usize;
    let oa_toa = decode_digits(&hex, i + 1, 2)?;
    let oa_start = i + 2;
    let oa_octets = (oa_digits + 1) / 2;
    i = oa_start + oa_octets;

    let number = if oa_toa & 0x70 == 0x50 {
        // Alphanumeric sender (7-bit packed); decode it as text.
        // Alphanumeric senders pack 4 bits per character: the length field
        // counts semi-octets, so the number of septets is `digits * 4 / 7`.
        let mut escape = false;
        let raw = decoded_octets(&hex, oa_start, oa_octets)?;
        let septets = unpack_septets(&raw, (oa_digits * 4) / 7);
        decode_septets(&septets, &mut escape)
    } else {
        decode_address(&hex, oa_start, oa_digits)
    };

    let _pid = decode_digits(&hex, i, 2)?;
    let dcs = decode_digits(&hex, i + 1, 2)?;
    i += 2;

    // TP-SCTS: YY MM DD HH MM SS TZ, two digits per octet, first digit in the
    // low nibble (the same semi-octet order as the address). The timezone
    // octet is quarters of an hour, which the UI's `+32` display also shows.
    let mut scts = [0u8; 7];
    for (k, slot) in scts.iter_mut().enumerate() {
        let byte = decode_digits(&hex, i + k, 2)?;
        let (tens, units) = (byte & 0x0F, byte >> 4);
        if tens > 9 || units > 9 {
            return None;
        }
        *slot = tens * 10 + units;
    }
    i += 7;
    let time = format!(
        "{:02}/{:02}/{:02},{:02}:{:02}:{:02}",
        scts[0], scts[1], scts[2], scts[3], scts[4], scts[5]
    );

    // DCS bits 3..2: 00 = GSM 7-bit, 01 = 8-bit, 10 = UCS-2 (3GPP 23.038).
    // The MT5700M's voice/SMS profile answers 0x08 for UCS-2, which is why the
    // predicate must test both bits, not just the 0x04 one.
    let ucs2 = (dcs >> 2) & 0x03 == 0x02;
    let udl = decode_digits(&hex, i, 2)? as usize;
    i += 1;
    let ud_octets = if ucs2 { udl } else { (udl * 7 + 7) / 8 };
    let ud = decoded_octets(&hex, i, ud_octets)?;

    // User-data header (concatenation IE) sits in front of the payload.
    let mut concat = None;
    let mut body = ud.clone();
    let mut header_octets = 0usize;
    if udhi && ud.len() >= 1 {
        let udhl = ud[0] as usize;
        header_octets = (udhl + 1).min(ud.len());
        let hdr = &ud[..header_octets];
        let mut p = 1usize;
        while p + 1 < hdr.len() {
            let iei = hdr[p];
            let iedl = hdr[p + 1] as usize;
            if p + 2 + iedl > hdr.len() {
                break;
            }
            if iei == 0x00 && iedl == 0x03 {
                concat = Some((hdr[p + 2], hdr[p + 3], hdr[p + 4]));
            }
            p += 2 + iedl;
        }
        body = ud[header_octets..].to_vec();
    }

    let text = if ucs2 {
        // UCS-2: big-endian pairs.
        let mut s = String::new();
        let mut k = 0usize;
        while k + 1 < body.len() {
            let cp = ((body[k] as u32) << 8) | body[k + 1] as u32;
            if let Some(c) = char::from_u32(cp) {
                s.push(c);
            }
            k += 2;
        }
        s
    } else {
        let septets = unpack_septets(&body, udl.saturating_sub((header_octets * 8 + 6) / 7));
        let mut escape = false;
        decode_septets(&septets, &mut escape)
    };

    if number.is_empty() && text.is_empty() {
        return None;
    }
    Some(DeliverSms {
        number,
        text,
        time,
        concat,
    })
}

/// Read `count` bytes starting at byte offset `start` of a hex string.
fn decoded_octets(hex: &[u8], start: usize, count: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(count);
    for k in 0..count {
        out.push(hex_byte(hex, start + k)?);
    }
    Some(out)
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

    #[test]
    fn message_stats_agree_with_the_encoder() {
        // The compose hint must never disagree with what a send produces.
        let mut cases: Vec<String> = vec![
            String::new(),
            "hello".to_string(),
            "a".repeat(160),
            "a".repeat(161),
            "a".repeat(306),
            "你好".to_string(),
            "你".repeat(70),
            "你".repeat(71),
            "你".repeat(134),
        ];
        for text in cases.drain(..) {
            let stats = message_stats(&text);
            let sent = encode("+8613800138000", &text);
            let expected_parts = if text.is_empty() { 0 } else { sent.len() };
            assert_eq!(stats.parts, expected_parts, "parts for {} chars", text.chars().count());
            assert_eq!(stats.chars, text.chars().count());
            let expect_encoding = if gsm7_septets(&text).is_some() { "7bit" } else { "UCS2" };
            assert_eq!(stats.encoding, expect_encoding);
        }
        // 161 GSM-7 characters split into 2 parts; 71 UCS-2 characters too.
        assert_eq!(message_stats(&"a".repeat(161)).parts, 2);
        assert_eq!(message_stats(&"你".repeat(71)).parts, 2);
        assert_eq!(message_stats("你好").parts, 1);
        assert_eq!(message_stats("你好").encoding, "UCS2");
        assert_eq!(message_stats("").parts, 0);
    }

}

#[cfg(test)]
mod deliver_tests {
    use super::*;

    /// +8613800138000 / 2026-08-26 14:28:36 (+8h) / "hello" / GSM 7-bit.
    const HELLO: &str = "00040D91683108108300F000006280624182632305E8329BFD06";
    /// Same envelope, UCS-2 payload: "你好".
    const UCS2: &str = "00040D91683108108300F0000862806241826323044F60597D";
    /// Same envelope with the concatenation header (ref 0xAB, part 1 of 2).
    const CONCAT: &str = "00440D91683108108300F00000628062418263230905C060250800D069";

    #[test]
    fn decodes_a_seven_bit_deliver() {
        let sms = decode_deliver(HELLO).expect("deliver");
        assert_eq!(sms.number, "8613800138000");
        assert_eq!(sms.text, "hello");
        assert_eq!(sms.time, "26/08/26,14:28:36");
        assert_eq!(sms.concat, None);
    }

    #[test]
    fn decodes_ucs2_and_the_udh() {
        let sms = decode_deliver(UCS2).expect("deliver");
        assert_eq!(sms.text, "你好");
        assert_eq!(sms.number, "8613800138000");
        let parts = decode_deliver(CONCAT).expect("deliver");
        assert_eq!(parts.text, "hi");
        assert_eq!(parts.concat, Some((0xAB, 2, 1)));
    }

    #[test]
    fn rejects_submits_and_garbage() {
        // MTI 01 is an SMS-SUBMIT, not something a list can render.
        assert!(decode_deliver("00010191683108108300F000006280624182632305E8329BFD06").is_none());
        assert!(decode_deliver("OK").is_none());
        assert!(decode_deliver("").is_none());
        assert!(decode_deliver("ZZZZ").is_none());
    }

    #[test]
    fn seven_bit_alphabet_matches_the_encoder() {
        // The alphabet table must stay in step with `gsm7_encode`: every entry
        // re-encodes to its own index.
        for (i, ch) in GSM7_ALPHABET.iter().enumerate() {
            assert_eq!(
                gsm7_encode(*ch),
                Some(i as u8),
                "alphabet entry {} ({:?}) does not round-trip",
                i,
                ch
            );
        }
    }
}
