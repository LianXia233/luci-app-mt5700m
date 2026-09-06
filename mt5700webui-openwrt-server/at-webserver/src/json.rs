//! Minimal JSON reader/writer. Std-only replacement for the shell pipeline
//! `json_init / json_dump | ubus | jsonfilter` and for the Python `json`
//! module usage in the WebSocket protocol.
//!
//! Scope is deliberately tiny: the AT daemon reply, the WS auth frame and the
//! pseudo-command SCHED payload are the only JSON documents in play.

use std::collections::BTreeMap;
use std::fmt::Write as _;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Num(String),
    Str(String),
    Arr(Vec<Value>),
    Obj(BTreeMap<String, Value>),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Obj(m) => m.get(key),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Value::Num(n) => n.parse().ok(),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn dump(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }

    fn write(&self, out: &mut String) {
        match self {
            Value::Null => out.push_str("null"),
            Value::Bool(true) => out.push_str("true"),
            Value::Bool(false) => out.push_str("false"),
            Value::Num(n) => out.push_str(n),
            Value::Str(s) => escape_into(s, out),
            Value::Arr(items) => {
                out.push('[');
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    v.write(out);
                }
                out.push(']');
            }
            Value::Obj(map) => {
                out.push('{');
                for (i, (k, v)) in map.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    escape_into(k, out);
                    out.push(':');
                    v.write(out);
                }
                out.push('}');
            }
        }
    }
}

pub fn escape_into(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

pub fn escape(s: &str) -> String {
    let mut out = String::new();
    escape_into(s, &mut out);
    out
}

pub fn str_val(s: &str) -> Value {
    Value::Str(s.to_string())
}

pub fn num_val<T: std::fmt::Display>(n: T) -> Value {
    Value::Num(n.to_string())
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

/// Parse a JSON document. Returns None on malformed input.
pub fn parse(input: &str) -> Option<Value> {
    let mut p = Parser {
        bytes: input.as_bytes(),
        pos: 0,
    };
    p.ws();
    let v = p.value()?;
    p.ws();
    if p.pos != p.bytes.len() {
        return None;
    }
    Some(v)
}

impl<'a> Parser<'a> {
    fn ws(&mut self) {
        while self.pos < self.bytes.len()
            && matches!(self.bytes[self.pos], b' ' | b'\t' | b'\n' | b'\r')
        {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn expect(&mut self, b: u8) -> Option<()> {
        if self.peek() == Some(b) {
            self.pos += 1;
            Some(())
        } else {
            None
        }
    }

    fn value(&mut self) -> Option<Value> {
        match self.peek()? {
            b'n' => self.lit("null", Value::Null),
            b't' => self.lit("true", Value::Bool(true)),
            b'f' => self.lit("false", Value::Bool(false)),
            b'"' => self.string().map(Value::Str),
            b'[' => self.array(),
            b'{' => self.object(),
            _ => self.number(),
        }
    }

    fn lit(&mut self, word: &str, v: Value) -> Option<Value> {
        if self.bytes[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            Some(v)
        } else {
            None
        }
    }

    fn string(&mut self) -> Option<String> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            match self.peek()? {
                b'"' => {
                    self.pos += 1;
                    return Some(out);
                }
                b'\\' => {
                    self.pos += 1;
                    match self.peek()? {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'u' => {
                            let hex = self.bytes.get(self.pos + 1..self.pos + 5)?;
                            let code = u32::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?;
                            self.pos += 4;
                            // Handle surrogate pairs for UCS2 SMS payloads.
                            if (0xD800..0xDC00).contains(&code) {
                                if self.bytes.get(self.pos + 1) == Some(&b'\\')
                                    && self.bytes.get(self.pos + 2) == Some(&b'u')
                                {
                                    let hex2 =
                                        self.bytes.get(self.pos + 3..self.pos + 7)?;
                                    let low = u32::from_str_radix(
                                        std::str::from_utf8(hex2).ok()?,
                                        16,
                                    )
                                    .ok()?;
                                    if (0xDC00..0xE000).contains(&low) {
                                        self.pos += 6;
                                        let c = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                                        out.push(char::from_u32(c)?);
                                        self.pos += 1;
                                        continue;
                                    }
                                }
                                out.push('\u{FFFD}');
                            } else {
                                out.push(char::from_u32(code)?);
                            }
                        }
                        _ => return None,
                    }
                    self.pos += 1;
                }
                _ => {
                    let start = self.pos;
                    while self.pos < self.bytes.len()
                        && self.bytes[self.pos] != b'"'
                        && self.bytes[self.pos] != b'\\'
                    {
                        self.pos += 1;
                    }
                    out.push_str(std::str::from_utf8(&self.bytes[start..self.pos]).ok()?);
                }
            }
        }
    }

    fn array(&mut self) -> Option<Value> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Some(Value::Arr(items));
        }
        loop {
            self.ws();
            items.push(self.value()?);
            self.ws();
            match self.peek()? {
                b',' => self.pos += 1,
                b']' => {
                    self.pos += 1;
                    return Some(Value::Arr(items));
                }
                _ => return None,
            }
        }
    }

    fn object(&mut self) -> Option<Value> {
        self.expect(b'{')?;
        let mut map = BTreeMap::new();
        self.ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Some(Value::Obj(map));
        }
        loop {
            self.ws();
            let key = self.string()?;
            self.ws();
            self.expect(b':')?;
            self.ws();
            let val = self.value()?;
            map.insert(key, val);
            self.ws();
            match self.peek()? {
                b',' => self.pos += 1,
                b'}' => {
                    self.pos += 1;
                    return Some(Value::Obj(map));
                }
                _ => return None,
            }
        }
    }

    fn number(&mut self) -> Option<Value> {
        let start = self.pos;
        while self.pos < self.bytes.len()
            && matches!(self.bytes[self.pos],
                b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E')
        {
            self.pos += 1;
        }
        if start == self.pos {
            return None;
        }
        Some(Value::Num(
            std::str::from_utf8(&self.bytes[start..self.pos]).ok()?.to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ubus_reply() {
        let v = parse(r#"{"status":"success","response":"OK\r\n"}"#).unwrap();
        assert_eq!(v.get("status").unwrap().as_str(), Some("success"));
        assert_eq!(v.get("response").unwrap().as_str(), Some("OK\r\n"));
    }

    #[test]
    fn roundtrips_escapes() {
        let v = Value::Str("a\"b\\c\n\u{1}".into());
        let s = v.dump();
        assert_eq!(parse(&s).unwrap(), v);
    }

    #[test]
    fn surrogate_pair() {
        let v = parse(r#""😀""#).unwrap();
        assert_eq!(v.as_str(), Some("\u{1F600}"));
    }
}
