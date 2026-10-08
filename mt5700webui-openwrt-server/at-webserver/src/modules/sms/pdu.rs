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
//!
//! A PDU handed to `AT+CMGS` is the service-centre field followed by the TPDU,
//! and `AT+CMGS=<len>` counts the TPDU only (3GPP 27.005). The centre comes
//! from `+CSCA?` (the value the settings page writes); when the modem reports
//! none, the field is the single octet `00`, which means "use the centre stored
//! in the SIM" — the same fallback the send page used to take.

use std::sync::atomic::{AtomicU8, Ordering};

/// A fully encoded SMS-SUBMIT PDU ready to be passed to `AT+CMGS=<len>`.
#[derive(Debug, Clone)]
pub struct SmsPdu {
    /// `AT+CMGS=<len>`: the TPDU length in **octets**. 3GPP 27.005 counts the
    /// TPDU alone, so the service-centre prefix in `hex` is not included.
    pub length: u32,
    /// The complete PDU in hex — service-centre field followed by the TPDU,
    /// exactly what follows `AT+CMGS=<len>` (before the trailing Ctrl-Z).
    pub hex: String,
}

impl SmsPdu {
    /// Wrap one TPDU with the service-centre field that must precede it.
    fn new(sca: &[u8], tpdu: Vec<u8>) -> Self {
        let length = tpdu.len() as u32;
        let hex: String = sca
            .iter()
            .chain(tpdu.iter())
            .map(|b| format!("{:02X}", b))
            .collect();
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

/// The concatenation UDH (8-bit reference, 3GPP 23.040 9.2.3.24):
/// `UDHL=5, IEI=0x00, IE length 3, reference, total, sequence`.
fn concat_udh(reference: u8, total: usize, seq: usize) -> [u8; 6] {
    [0x05, 0x00, 0x03, reference, total as u8, seq as u8 + 1]
}

/// Septets a header occupies: `ceil(octets * 8 / 7)` — 6 octets are 7 septets
/// (48 bits plus one padding bit). 3GPP 23.040: after a UDH, a 7-bit payload
/// starts on the next septet boundary.
fn header_septets(header: &[u8]) -> usize {
    (header.len() * 8 + 6) / 7
}

/// Pack a header's octets followed by a 7-bit payload into one user-data
/// buffer: the header sits at bits 0.., the payload resumes on the next septet
/// boundary. Unused bits (the padding bit and the final partial octet) stay 0.
fn pack_udh_and_septets(header: &[u8], septets: &[u8]) -> Vec<u8> {
    let start = header_septets(header);
    let total_bits = start * 7 + septets.len() * 7;
    let mut out = vec![0u8; (total_bits + 7) / 8];
    for (i, &b) in header.iter().enumerate() {
        out[i] |= b; // the UDH itself is byte-aligned at the start
    }
    for (i, &s) in septets.iter().enumerate() {
        let bit = start * 7 + i * 7;
        let (byte, shift) = (bit / 8, bit % 8);
        let value = ((s & 0x7F) as u16) << shift;
        out[byte] |= (value & 0xFF) as u8;
        if let Some(next) = out.get_mut(byte + 1) {
            *next |= (value >> 8) as u8;
        }
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

/// Pack digits into swapped semi-octets (low nibble first), padding an odd
/// trailing digit with `F` — the Address-Value layout both the service-centre
/// field and the destination use.
fn pack_digits(digits: &str) -> Vec<u8> {
    let chars: Vec<char> = digits.chars().filter(|c| c.is_ascii_digit()).collect();
    let mut out = Vec::with_capacity((chars.len() + 1) / 2);
    let mut i = 0;
    while i < chars.len() {
        let hi = chars[i].to_digit(16).unwrap_or(0) as u8;
        let lo = chars
            .get(i + 1)
            .and_then(|c| c.to_digit(16))
            .unwrap_or(15) as u8;
        out.push((lo << 4) | hi); // nibbles swapped, low nibble first
        i += 2;
    }
    out
}

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
    let mut out = Vec::with_capacity(2 + usize::from(nibbles) / 2);
    out.push(nibbles);
    out.push(toa);
    out.extend_from_slice(&pack_digits(&digits));
    out
}

/// The service-centre field every PDU starts with: length octet (which counts
/// the TOA octet too), TOA 0x91 and the swapped digits. `None`/no digits yields
/// the single octet `00`, i.e. "whatever centre the SIM holds".
fn service_center(center: Option<&str>) -> Vec<u8> {
    let digits: String = center
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        return vec![0x00];
    }
    let octets = (digits.len() + 1) / 2;
    let mut out = Vec::with_capacity(2 + octets);
    out.push((octets + 1) as u8); // the length octet includes the TOA octet
    out.push(0x91); // the page always sent the centre as an international number
    out.extend_from_slice(&pack_digits(&digits));
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

/// Build the final list of PDUs for `number`/`text`, each prefixed with the
/// service-centre field (`center`, as reported by `+CSCA?`; `None` means `00`).
/// Returns at least one PDU; long text is split into concatenated parts
/// carrying a UDHI header.
pub fn encode(number: &str, text: &str, center: Option<&str>) -> Vec<SmsPdu> {
    let refnum = next_ref();
    let sca = service_center(center);

    if let Some(septets) = gsm7_septets(text) {
        if septets.len() <= GSM7_SINGLE_MAX {
            let packed = pack_septets(&septets);
            let tpdu = assemble_tpdu(number, 0x00, septets.len() as u32, false, &packed);
            return vec![SmsPdu::new(&sca, tpdu)];
        }
        let total_parts = part_count(septets.len(), GSM7_SINGLE_MAX, GSM7_PART_MAX);
        let mut pdus = Vec::with_capacity(total_parts);
        for seq in 0..total_parts {
            let start = seq * GSM7_PART_MAX;
            let end = (start + GSM7_PART_MAX).min(septets.len());
            let payload = &septets[start..end];
            let udh = concat_udh(refnum, total_parts, seq);
            let ud = pack_udh_and_septets(&udh, payload);
            // TP-UDL counts septets, header included.
            let udl = (header_septets(&udh) + payload.len()) as u32;
            let tpdu = assemble_tpdu(number, 0x00, udl, true, &ud);
            pdus.push(SmsPdu::new(&sca, tpdu));
        }
        return pdus;
    }

    // UCS-2 path.
    let octets = ucs2_be(text);
    if octets.len() <= UCS2_SINGLE_MAX {
        let tpdu = assemble_tpdu(number, 0x08, octets.len() as u32, false, &octets);
        return vec![SmsPdu::new(&sca, tpdu)];
    }
    let total_parts = part_count(octets.len(), UCS2_SINGLE_MAX, UCS2_PART_MAX);
    let mut pdus = Vec::with_capacity(total_parts);
    for seq in 0..total_parts {
        let start = seq * UCS2_PART_MAX;
        let end = (start + UCS2_PART_MAX).min(octets.len());
        let mut ud = concat_udh(refnum, total_parts, seq).to_vec();
        ud.extend_from_slice(&octets[start..end]);
        // UCS-2 is octet-aligned, so TP-UDL counts octets, header included.
        let tpdu = assemble_tpdu(number, 0x08, ud.len() as u32, true, &ud);
        pdus.push(SmsPdu::new(&sca, tpdu));
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

/// Unpack `count` septets from GSM 7-bit packed octets, starting at septet
/// `start` (0 for a payload with no header, `ceil(header_octets * 8 / 7)` when
/// a UDH shifted the payload off the byte boundary).
fn unpack_septets_at(octets: &[u8], start: usize, count: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(count);
    let mut bit = start * 7;
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

/// Unpack septets from the start of the octets.
fn unpack_septets(octets: &[u8], count: usize) -> Vec<u8> {
    unpack_septets_at(octets, 0, count)
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
    let hi = (*hex.get(i * 2)? as char).to_digit(16)?;
    let lo = (*hex.get(i * 2 + 1)? as char).to_digit(16)?;
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
    let smsc_len = hex_byte(&hex, 0)? as usize;
    let mut i = 1 + smsc_len;
    let first_octet = hex_byte(&hex, i)?;
    i += 1;
    // TP-MTI must be 00 (SMS-DELIVER); 01 is a SUBMIT we cannot render.
    if first_octet & 0x03 != 0 {
        return None;
    }
    let udhi = first_octet & 0x40 != 0;

    // TP-OA: length (digits), type-of-address, then the packed digits.
    let oa_digits = hex_byte(&hex, i)? as usize;
    let oa_toa = hex_byte(&hex, i + 1)?;
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

    let _pid = hex_byte(&hex, i)?;
    let dcs = hex_byte(&hex, i + 1)?;
    i += 2;

    // TP-SCTS: YY MM DD HH MM SS TZ, two digits per octet, first digit in the
    // low nibble (the same semi-octet order as the address). The timezone
    // octet is quarters of an hour, which the UI's `+32` display also shows.
    let mut scts = [0u8; 7];
    for (k, slot) in scts.iter_mut().enumerate() {
        let byte = hex_byte(&hex, i + k)?;
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
    let udl = hex_byte(&hex, i)? as usize;
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
        // UCS-2 is octet-aligned, so the header is simply skipped.
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
        // 7-bit: the UDH is a whole number of octets but the payload continues
        // on a *septet* boundary, so 6 header octets consume ceil(48 / 7) = 7
        // septets (padding included). Slicing bytes instead would shift every
        // character by one bit.
        let start = if udhi {
            (header_octets * 8 + 6) / 7
        } else {
            0
        };
        let septets = unpack_septets_at(&ud, start, udl.saturating_sub(start));
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

    /// Read a hex TPDU back into octets (test helper).
    fn hex_to_bytes(hex: &str) -> Vec<u8> {
        hex.as_bytes()
            .chunks(2)
            .map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap())
            .collect()
    }

    /// The TPDU inside a PDU: skip the service-centre field (a length octet
    /// plus that many octets) so the caller sees `[FO][MR][DA-len][TOA]…`.
    fn tpdu_of(pdu_hex: &str) -> Vec<u8> {
        let pdu = hex_to_bytes(pdu_hex);
        let sca_octets = pdu[0] as usize;
        pdu[1 + sca_octets..].to_vec()
    }

    /// Re-wrap an SMS-SUBMIT's user data as an SMS-DELIVER — the way the same
    /// message comes back from the network. The submit's destination becomes
    /// the sender, the timestamp is fixed.
    fn deliver_from_submit(submit_hex: &str) -> String {
        let t = tpdu_of(submit_hex);
        let oa_octets = (t[2] as usize + 1) / 2; // FO, MR, DA-len, TOA, DA…
        let dcs = t[5 + oa_octets];
        let udl = t[6 + oa_octets];
        let ud = &t[7 + oa_octets..];
        // SMS-DELIVER first octet: MTI 00, TP-UDHI copied from the submit (the
        // only bit that has to survive the direction change here).
        let mut out: Vec<u8> = vec![0x00, 0x04 | (t[0] & 0x40), t[2], t[3]];
        out.extend_from_slice(&t[4..4 + oa_octets]);
        out.push(0x00); // TP-PID
        out.push(dcs);
        out.extend_from_slice(&[0x62, 0x80, 0x62, 0x41, 0x82, 0x63, 0x23]); // SCTS
        out.push(udl);
        out.extend_from_slice(ud);
        out.iter().map(|b| format!("{:02X}", b)).collect()
    }

    #[test]
    fn single_ascii_pdu_shapes() {
        let pdus = encode("+8613800138000", "hello", None);
        assert_eq!(pdus.len(), 1);
        // `00` service centre (the SIM's own) + SMS-SUBMIT, no UDHI.
        assert!(pdus[0].hex.starts_with("0001"));
        // 14 digits -> address-length nibble 0x0E, TOA 0x91 (intl).
        assert!(pdus[0].hex.contains("0E91"));
        // `AT+CMGS=<len>` takes the TPDU octet count: the whole PDU minus the
        // single-octet `00` service-centre field.
        assert_eq!(pdus[0].length as usize, pdus[0].hex.len() / 2 - 1);
    }

    #[test]
    fn service_centre_field_is_built_from_the_configuration() {
        let without = encode("+8613800138000", "hello", None);
        assert!(without[0].hex.starts_with("00"));

        let with = encode("+8613800138000", "hello", Some("+8613800138000"));
        // 13 digits -> 14 nibbles -> 7 octets, so the length octet counts the
        // TOA too: 8. Field = 08 91 68 31 08 10 83 00 F0 (9 octets).
        assert!(with[0].hex.starts_with("0891683108108300F0"), "{}", with[0].hex);
        assert_eq!(with[0].length as usize, with[0].hex.len() / 2 - 9);
        // Same TPDU in both cases; only the service-centre field differs.
        assert_eq!(with[0].length, without[0].length);
        assert!(with[0].hex.ends_with(&without[0].hex[2..]));
    }

    #[test]
    fn ucs2_pdu_for_chinese() {
        let pdus = encode("+8613800138000", "你好", None);
        assert_eq!(pdus.len(), 1);
        assert!(pdus[0].hex.contains("08")); // DCS UCS-2
        assert_eq!(pdus[0].length as usize, pdus[0].hex.len() / 2 - 1);
        // UDL byte 0x04 followed by the 4 UTF-16BE data octets.
        assert!(pdus[0].hex.ends_with("044F60597D"));
        assert!(pdus[0].hex.contains("4F60597D"));
    }

    #[test]
    fn multipart_split_adds_udhi_and_a_standard_header() {
        let text = "x".repeat(200);
        let pdus = encode("13800138000", &text, None);
        assert!(pdus.len() > 1, "expected >1 part, got {}", pdus.len());
        for (i, p) in pdus.iter().enumerate() {
            assert!(p.hex.starts_with("0041")); // 00 SC + SMS-SUBMIT + UDHI
            // The PDU is the TPDU plus the single-octet `00` service centre.
            assert_eq!(p.length as usize, p.hex.len() / 2 - 1);
            // UDHL=5, IEI=0x00, IE length 3, then total and this sequence.
            let t = tpdu_of(&p.hex);
            let oa_octets = (t[2] as usize + 1) / 2;
            let udl = t[6 + oa_octets] as usize;
            let ud = &t[7 + oa_octets..];
            assert_eq!(&ud[..4], &[0x05, 0x00, 0x03, ud[3]], "part {}", i + 1);
            assert_eq!(ud[4], pdus.len() as u8); // total parts
            assert_eq!(ud[5], i as u8 + 1); // this part
            // TP-UDL counts septets, header included: 7 for the 6-octet UDH
            // plus this part's payload (153 septets per full part).
            let payload = (text.len() - i * GSM7_PART_MAX).min(GSM7_PART_MAX);
            assert_eq!(udl, 7 + payload, "part {}", i + 1);
            // One reference for every part of this message.
            assert_eq!(ud[3], {
                let first = tpdu_of(&pdus[0].hex);
                let first_oa = (first[2] as usize + 1) / 2;
                first[7 + first_oa + 3]
            });
        }
    }

    #[test]
    fn multipart_round_trip_through_the_list_parser() {
        // The same implementation writes the header and reads it back: encode a
        // long message, present each part as an SMS-DELIVER would arrive, and
        // let the list parser merge them into one message again.
        let text = "The quick brown fox jumps over the lazy dog. ".repeat(6);
        let parts = encode("+8613800138000", &text, None);
        assert!(parts.len() > 1, "expected a split message");
        let mut cmgl = String::from("AT+CMGL=4\r\n");
        for (i, part) in parts.iter().enumerate() {
            let deliver = deliver_from_submit(&part.hex);
            cmgl.push_str(&format!(
                "+CMGL: {},1,,{}\r\n{}\r\n",
                i + 1,
                deliver.len() / 2,
                deliver
            ));
        }
        cmgl.push_str("OK");
        let list = crate::modules::sms::parser::parse_cmgl(&cmgl);
        assert_eq!(list.len(), 1, "the parts must merge into one message");
        assert_eq!(list[0].content, text);
        assert!(list[0].is_concatenated);
        assert_eq!(list[0].concatenated_total, Some(parts.len() as u8));
    }

    #[test]
    fn multipart_ucs2_split() {
        let text = "中".repeat(90); // 180 octets -> split UCS-2
        let pdus = encode("13800138000", &text, None);
        assert!(pdus.len() > 1);
        assert!(pdus[0].hex.starts_with("0041"));
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
            let sent = encode("+8613800138000", &text, None);
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
    /// Same envelope with a standard concatenation UDH (ref 0xAB, part 1 of 2)
    /// and the 7-bit payload "hi" resuming after the header's septet boundary.
    const CONCAT: &str = "00440D91683108108300F000006280624182632309050003AB0201D069";

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
