//! USSD (`AT+CUSD`, manual 5.21/5.22): one codec for both directions.
//!
//! The WebUI's USSD panel used to pack the code itself: it built
//! `AT+CUSD=1,"<gsm7 hex>",15`, unpacked the network's answer from `+CUSD:`
//! URCs (GSM 7-bit low-bits-first, UCS-2, 8-bit), mapped the `<m>` result codes
//! to Chinese copy and validated the input. All of that lives here now — the
//! page sends `{code}` and renders the reply object.
//!
//! USSD belongs to the messaging module (the panel is on the SMS page and the
//! feature shares `+CUSD`'s session semantics with SMS on the same modem).

use crate::core::json::{self, Value};
use std::collections::BTreeMap;

/// The manual's `AT+CUSD=<n>,<str>,<dcs>` DCS values (5.21.3).
pub const DCS_GSM7: u8 = 15;
pub const DCS_8BIT: u8 = 68;
pub const DCS_UCS2: u8 = 72;
/// The panel's own input limit, kept as its copy for it.
pub const MAX_CODE_CHARS: usize = 160;

/// `AT+CUSD=2` — release the USSD session (5.21.3, n=2).
pub const CANCEL: &str = "AT+CUSD=2";

/// Pack text into GSM 7-bit septets, low bits first (3GPP 23.038), hex.
///
/// This is the form the firmware expects in the `<str>` argument; the manual's
/// own example (`"AAD86C3602"` = `*133#`) is exactly this encoding.
pub fn pack_gsm7(text: &str) -> String {
    let mut octets: Vec<u8> = Vec::new();
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    for ch in text.chars() {
        acc |= ((ch as u32) & 0x7f) << bits;
        bits += 7;
        while bits >= 8 {
            octets.push((acc & 0xff) as u8);
            acc >>= 8;
            bits -= 8;
        }
    }
    if bits > 0 {
        octets.push((acc & 0xff) as u8);
    }
    let mut out = String::with_capacity(octets.len() * 2);
    for b in octets {
        out.push_str(&format!("{:02X}", b));
    }
    out
}

/// Unpack GSM 7-bit septets (low bits first) back to text.
pub fn unpack_gsm7(hex: &str) -> String {
    let mut out = String::new();
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    let bytes = hex.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        let Some(byte) = hex_byte(hex, i) else {
            return out;
        };
        acc |= (byte as u32) << bits;
        bits += 8;
        while bits >= 7 {
            out.push(((acc & 0x7f) as u8) as char);
            acc >>= 7;
            bits -= 7;
        }
        i += 2;
    }
    // The final septet's zero padding can surface as trailing NULs.
    while out.ends_with('\0') {
        out.pop();
    }
    out
}

/// One hex byte at `index` (even), `None` when the pair is not hex.
fn hex_byte(hex: &str, index: usize) -> Option<u8> {
    let pair = hex.get(index..index + 2)?;
    u8::from_str_radix(pair, 16).ok()
}

fn is_hex(s: &str) -> bool {
    !s.is_empty() && s.len() % 2 == 0 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Decode a `<str>` field by its `<dcs>`; a modem that answers plain text is
/// returned as-is (the panel's rule).
pub fn decode(str_field: &str, dcs: u8) -> String {
    let raw = str_field.trim();
    if !is_hex(raw) {
        return raw.to_string();
    }
    match dcs {
        DCS_UCS2 => ucs2_to_string(raw),
        DCS_8BIT => {
            let mut out = String::new();
            let bytes = raw.as_bytes();
            let mut i = 0;
            while i + 1 < bytes.len() {
                match hex_byte(raw, i) {
                    Some(b) => out.push(b as char),
                    None => break,
                }
                i += 2;
            }
            out
        }
        _ => unpack_gsm7(raw),
    }
}

/// Decode a hex UCS-2 string properly (surrogate pairs included).
fn ucs2_to_string(hex: &str) -> String {
    let mut units: Vec<u16> = Vec::new();
    let mut i = 0;
    while i + 3 < hex.len() {
        match (hex_byte(hex, i), hex_byte(hex, i + 2)) {
            (Some(hi), Some(lo)) => units.push(((hi as u16) << 8) | lo as u16),
            _ => break,
        }
        i += 4;
    }
    String::from_utf16_lossy(&units)
}

/// Manual 5.22.3 `<m>` result codes → the copy the panel shows.
pub fn result_text(m: i64) -> String {
    match m {
        0 => "网络无需回复".to_string(),
        1 => "网络等待进一步输入".to_string(),
        2 => "会话已被网络释放".to_string(),
        3 => "其他客户端已响应".to_string(),
        4 => "操作不支持".to_string(),
        5 => "网络超时".to_string(),
        other => format!("状态 {}", other),
    }
}

/// `{m, mText, text, needsReply}` — the panel's reply object.
pub struct Reply {
    pub m: i64,
    pub text: String,
    pub needs_reply: bool,
}

impl Reply {
    pub fn to_json(&self) -> Value {
        let mut m: BTreeMap<String, Value> = BTreeMap::new();
        m.insert("m".to_string(), json::num_val(self.m));
        m.insert("mText".to_string(), json::str_val(&result_text(self.m)));
        m.insert("text".to_string(), json::str_val(&self.text));
        m.insert("needsReply".to_string(), Value::Bool(self.needs_reply));
        Value::Obj(m)
    }
}

/// Parse a `+CUSD: <m>[,<str>,<dcs>]` line — an inline command answer and an
/// unsolicited network reply have the same shape.
pub fn parse_reply(line: &str) -> Option<Reply> {
    let idx = line.find("+CUSD:")?;
    let body = line[idx + "+CUSD:".len()..].trim();
    let mut fields: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in body.chars() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => {
                fields.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    fields.push(cur.trim().to_string());
    let m: i64 = fields.first()?.trim().parse().ok()?;
    let text_field = fields.get(1).cloned().unwrap_or_default();
    let dcs = fields
        .get(2)
        .filter(|f| !f.is_empty())
        .and_then(|f| f.trim().parse::<u8>().ok())
        .unwrap_or(DCS_GSM7);
    let text = if text_field.is_empty() {
        String::new()
    } else {
        decode(&text_field, dcs)
    };
    Some(Reply {
        m,
        text,
        needs_reply: m == 1,
    })
}

/// Build the send command for a USSD code, or the rejection copy the panel
/// showed when it validated locally.
pub fn build_command(code: &str) -> Result<String, String> {
    let text = code.trim();
    if text.is_empty() {
        return Err("请输入 USSD 代码，例如 *133#".to_string());
    }
    if text.chars().count() > MAX_CODE_CHARS {
        return Err(format!("USSD 字符串最长 {} 个字符", MAX_CODE_CHARS));
    }
    if !text.chars().all(|c| c.is_ascii_digit() || matches!(c, '*' | '#' | '+')) {
        return Err("USSD 代码只能包含数字与 * # +".to_string());
    }
    Ok(format!("AT+CUSD=1,\"{}\",{}", pack_gsm7(text), DCS_GSM7))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_manual_example_round_trips() {
        // 手册 5.21 的示例：*133# 打包后正是 AAD86C3602。
        assert_eq!(pack_gsm7("*133#"), "AAD86C3602");
        assert_eq!(unpack_gsm7("AAD86C3602"), "*133#");
        assert_eq!(
            build_command("*133#").unwrap(),
            "AT+CUSD=1,\"AAD86C3602\",15"
        );
    }

    #[test]
    fn packing_is_septet_aligned_not_byte_aligned() {
        // Seven characters pack into 7 octets (49 bits), no trailing NUL.
        assert_eq!(pack_gsm7("1234567").len(), 14);
        assert_eq!(unpack_gsm7(&pack_gsm7("10086")), "10086");
        assert_eq!(unpack_gsm7(&pack_gsm7("*101#*2#")), "*101#*2#");
        // A plain-text answer is returned unchanged instead of being mangled.
        assert_eq!(decode("余额 12.30 元", DCS_GSM7), "余额 12.30 元");
    }

    #[test]
    fn decode_honours_the_dcs() {
        // UCS-2: 4E2D 6587 = 中文
        assert_eq!(decode("4E2D6587", DCS_UCS2), "中文");
        // 8-bit ASCII
        assert_eq!(decode("414243", DCS_8BIT), "ABC");
        // 7-bit default
        assert_eq!(decode(&pack_gsm7("12345"), DCS_GSM7), "12345");
    }

    #[test]
    fn replies_carry_the_manual_copy() {
        let r = parse_reply("[+CUSD] +CUSD: 1,\"AAD86C3602\",15").unwrap();
        assert_eq!(r.m, 1);
        assert_eq!(r.text, "*133#");
        assert!(r.needs_reply);
        assert_eq!(result_text(r.m), "网络等待进一步输入");

        // The panel's own fallback: no text, session released.
        let r = parse_reply("+CUSD: 2").unwrap();
        assert_eq!(r.m, 2);
        assert_eq!(r.text, "");
        assert!(!r.needs_reply);
        assert_eq!(result_text(2), "会话已被网络释放");

        assert!(parse_reply("OK").is_none());
        assert_eq!(result_text(9), "状态 9");
    }

    #[test]
    fn the_panels_validation_copy_is_preserved() {
        assert_eq!(
            build_command("  ").unwrap_err(),
            "请输入 USSD 代码，例如 *133#"
        );
        assert_eq!(
            build_command("call me").unwrap_err(),
            "USSD 代码只能包含数字与 * # +"
        );
        assert_eq!(
            build_command(&"1".repeat(161)).unwrap_err(),
            "USSD 字符串最长 160 个字符"
        );
        assert!(build_command("+8613800138000").is_ok());
    }

    #[test]
    fn cancel_is_the_manual_release_command() {
        assert_eq!(CANCEL, "AT+CUSD=2");
    }
}
