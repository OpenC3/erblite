// Copyright 2026 OpenC3, Inc.
// All Rights Reserved.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
// See LICENSE.md for more details.

//! JSON parsing and generation (`JSON.parse`, `JSON.generate`, `JSON.pretty_generate`).
//!
//! Floats are written with `Float#to_s` (as Ruby 3.4's bundled json 2.9.1 does).

use crate::error::{fail, JSON_GENERATOR_ERROR, JSON_NESTING_ERROR, JSON_PARSER_ERROR, R, RANGE_ERROR};
use crate::value::{RHash, Value};

/// Maximum nesting depth for parse and generate (the Ruby default).
const MAX_NESTING: usize = 100;

// ----- parsing ------------------------------------------------------------------------------

struct JsonParser<'a> {
    s: &'a [u8],
    pos: usize,
    symbolize: bool,
}

/// Parses JSON text. Objects become Hashes (String keys, or Symbols with `symbolize`).
pub fn parse(text: &str, symbolize: bool) -> R<Value> {
    let mut p = JsonParser {
        s: text.as_bytes(),
        pos: 0,
        symbolize,
    };
    p.skip_ws()?;
    if p.pos >= p.s.len() {
        return fail(JSON_PARSER_ERROR, "unexpected end of input");
    }
    let v = p.value(0)?;
    p.skip_ws()?;
    if p.pos < p.s.len() {
        return p.err("unexpected token at end of stream");
    }
    Ok(v)
}

impl JsonParser<'_> {
    fn err<T>(&self, msg: &str) -> R<T> {
        let rest: String = String::from_utf8_lossy(&self.s[self.pos.min(self.s.len())..])
            .chars()
            .take(20)
            .collect();
        fail(JSON_PARSER_ERROR, format!("{} '{}'", msg, rest))
    }

    fn skip_ws(&mut self) -> R<()> {
        loop {
            match self.s.get(self.pos) {
                Some(b' ' | b'\t' | b'\n' | b'\r') => self.pos += 1,
                Some(b'/') => match self.s.get(self.pos + 1) {
                    Some(b'*') => {
                        self.pos += 2;
                        loop {
                            if self.pos + 1 >= self.s.len() {
                                return self.err("unexpected end of input in comment");
                            }
                            if self.s[self.pos] == b'*' && self.s[self.pos + 1] == b'/' {
                                self.pos += 2;
                                break;
                            }
                            self.pos += 1;
                        }
                    }
                    Some(b'/') => {
                        while self.pos < self.s.len() && self.s[self.pos] != b'\n' {
                            self.pos += 1;
                        }
                    }
                    _ => return self.err("unexpected character"),
                },
                _ => return Ok(()),
            }
        }
    }

    fn value(&mut self, depth: usize) -> R<Value> {
        crate::stack::check()?;
        match self.s.get(self.pos) {
            None => fail(JSON_PARSER_ERROR, "unexpected end of input"),
            Some(b'{') => self.object(depth + 1),
            Some(b'[') => self.array(depth + 1),
            Some(b'"') => Ok(Value::string(self.string()?)),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => {
                for (word, v) in [
                    ("true", Value::Bool(true)),
                    ("false", Value::Bool(false)),
                    ("null", Value::Nil),
                ] {
                    if self.s[self.pos..].starts_with(word.as_bytes()) {
                        self.pos += word.len();
                        return Ok(v);
                    }
                }
                self.err("unexpected token")
            }
        }
    }

    fn check_depth(&self, depth: usize) -> R<()> {
        // the json gem accepts 101 levels when parsing (100 when generating)
        if depth > MAX_NESTING + 1 {
            return fail(JSON_NESTING_ERROR, format!("nesting of {} is too deep", depth));
        }
        Ok(())
    }

    fn object(&mut self, depth: usize) -> R<Value> {
        self.check_depth(depth)?;
        self.pos += 1;
        let mut h = RHash::new();
        self.skip_ws()?;
        if self.s.get(self.pos) == Some(&b'}') {
            self.pos += 1;
            return Ok(Value::hash(h));
        }
        loop {
            self.skip_ws()?;
            if self.s.get(self.pos) != Some(&b'"') {
                return self.err("expected object key, got");
            }
            let k = self.string()?;
            self.skip_ws()?;
            if self.s.get(self.pos) != Some(&b':') {
                return self.err("expected ':' after object key");
            }
            self.pos += 1;
            self.skip_ws()?;
            let v = self.value(depth)?;
            let key = if self.symbolize {
                Value::sym(&k)
            } else {
                Value::string(k)
            };
            h.insert(key, v);
            self.skip_ws()?;
            match self.s.get(self.pos) {
                Some(b',') => {
                    self.pos += 1;
                    continue;
                }
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Value::hash(h));
                }
                None => return fail(JSON_PARSER_ERROR, "unexpected end of input"),
                _ => return self.err("expected ',' or '}' after object value"),
            }
        }
    }

    fn array(&mut self, depth: usize) -> R<Value> {
        self.check_depth(depth)?;
        self.pos += 1;
        let mut items = Vec::new();
        self.skip_ws()?;
        if self.s.get(self.pos) == Some(&b']') {
            self.pos += 1;
            return Ok(Value::array(items));
        }
        loop {
            self.skip_ws()?;
            items.push(self.value(depth)?);
            self.skip_ws()?;
            match self.s.get(self.pos) {
                Some(b',') => {
                    self.pos += 1;
                    self.skip_ws()?;
                    if self.s.get(self.pos) == Some(&b']') {
                        return self.err("unexpected character");
                    }
                }
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Value::array(items));
                }
                None => return fail(JSON_PARSER_ERROR, "unexpected end of input"),
                _ => return self.err("expected ',' or ']' after array value"),
            }
        }
    }

    fn hex4(&mut self) -> R<u32> {
        let Some(h) = self.s.get(self.pos..self.pos + 4) else {
            return self.err("incomplete unicode character escape sequence");
        };
        let txt = std::str::from_utf8(h).unwrap_or("");
        if txt.len() != 4 || !txt.bytes().all(|b| b.is_ascii_hexdigit()) {
            return self.err("incomplete unicode character escape sequence");
        }
        self.pos += 4;
        Ok(u32::from_str_radix(txt, 16).unwrap_or(0))
    }

    fn string(&mut self) -> R<String> {
        self.pos += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let Some(&c) = self.s.get(self.pos) else {
                return fail(JSON_PARSER_ERROR, "unexpected end of input in string");
            };
            self.pos += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let Some(&e) = self.s.get(self.pos) else {
                        return fail(JSON_PARSER_ERROR, "unexpected end of input in string");
                    };
                    self.pos += 1;
                    let ch = match e {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\x08',
                        b'f' => '\x0c',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let hi = self.hex4()?;
                            if (0xD800..0xDC00).contains(&hi) && self.s[self.pos..].starts_with(b"\\u") {
                                self.pos += 2;
                                let lo = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&lo) {
                                    return self.err("invalid Unicode surrogate pair");
                                }
                                let cp = 0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00);
                                char::from_u32(cp).unwrap_or('\u{FFFD}')
                            } else {
                                match char::from_u32(hi) {
                                    Some(c) => c,
                                    None => return self.err("invalid Unicode lone surrogate"),
                                }
                            }
                        }
                        _ => {
                            self.pos -= 2;
                            return self.err("invalid escape character in string");
                        }
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                c if c < 0x20 => {
                    self.pos -= 1;
                    return self.err("invalid ASCII control character in string");
                }
                c => out.push(c),
            }
        }
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    fn number(&mut self) -> R<Value> {
        let start = self.pos;
        if self.s.get(self.pos) == Some(&b'-') {
            self.pos += 1;
        }
        let int_start = self.pos;
        while self.s.get(self.pos).is_some_and(|b| b.is_ascii_digit()) {
            self.pos += 1;
        }
        let int_len = self.pos - int_start;
        if int_len == 0 || (int_len > 1 && self.s[int_start] == b'0') {
            self.pos = start;
            return self.err("invalid number:");
        }
        let mut is_float = false;
        if self.s.get(self.pos) == Some(&b'.') {
            self.pos += 1;
            let f = self.pos;
            while self.s.get(self.pos).is_some_and(|b| b.is_ascii_digit()) {
                self.pos += 1;
            }
            if self.pos == f {
                self.pos = start;
                return self.err("invalid number:");
            }
            is_float = true;
        }
        if matches!(self.s.get(self.pos), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.s.get(self.pos), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            let f = self.pos;
            while self.s.get(self.pos).is_some_and(|b| b.is_ascii_digit()) {
                self.pos += 1;
            }
            if self.pos == f {
                self.pos = start;
                return self.err("invalid number:");
            }
            is_float = true;
        }
        let txt = std::str::from_utf8(&self.s[start..self.pos]).unwrap_or("0");
        if is_float {
            Ok(Value::Float(txt.parse::<f64>().unwrap_or(f64::NAN)))
        } else {
            match txt.parse::<i64>() {
                Ok(i) => Ok(Value::Int(i)),
                Err(_) => fail(RANGE_ERROR, format!("integer {} out of 64-bit range", txt)),
            }
        }
    }
}

// ----- generation ---------------------------------------------------------------------------

/// Compact JSON (`JSON.generate`, `to_json`).
pub fn generate(v: &Value) -> R<String> {
    let mut out = String::new();
    gen(v, &mut out, None, 0)?;
    Ok(out)
}

/// Indented JSON (`JSON.pretty_generate`).
pub fn pretty_generate(v: &Value) -> R<String> {
    let mut out = String::new();
    gen(v, &mut out, Some("  "), 0)?;
    Ok(out)
}

/// Appends a JSON string literal.
pub fn escape_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn newline_indent(out: &mut String, indent: Option<&str>, depth: usize) {
    if let Some(ind) = indent {
        out.push('\n');
        for _ in 0..depth {
            out.push_str(ind);
        }
    }
}

fn gen(v: &Value, out: &mut String, indent: Option<&str>, depth: usize) -> R<()> {
    crate::stack::check()?;
    match v {
        Value::Nil => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(i) => out.push_str(&i.to_string()),
        Value::Float(f) => {
            if !f.is_finite() {
                return fail(
                    JSON_GENERATOR_ERROR,
                    format!("{} not allowed in JSON", crate::format::float_to_s(*f)),
                );
            }
            out.push_str(&crate::format::float_to_s(*f));
        }
        Value::Str(s) => escape_str(s, out),
        Value::Sym(s) => escape_str(s, out),
        Value::Array(a) => {
            let d = depth + 1;
            if d > MAX_NESTING {
                return fail(JSON_NESTING_ERROR, format!("nesting of {} is too deep", d));
            }
            let items = a.borrow().clone();
            if items.is_empty() {
                out.push_str("[]");
                return Ok(());
            }
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline_indent(out, indent, d);
                gen(item, out, indent, d)?;
            }
            newline_indent(out, indent, depth);
            out.push(']');
        }
        Value::Hash(h) => {
            let d = depth + 1;
            if d > MAX_NESTING {
                return fail(JSON_NESTING_ERROR, format!("nesting of {} is too deep", d));
            }
            let pairs = h.borrow().pairs();
            if pairs.is_empty() {
                out.push_str("{}");
                return Ok(());
            }
            out.push('{');
            for (i, (k, val)) in pairs.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline_indent(out, indent, d);
                let key = match k {
                    Value::Str(s) => s.to_string(),
                    other => other.to_s()?,
                };
                escape_str(&key, out);
                out.push(':');
                if indent.is_some() {
                    out.push(' ');
                }
                gen(val, out, indent, d)?;
            }
            newline_indent(out, indent, depth);
            out.push('}');
        }
        other => escape_str(&other.to_s()?, out),
    }
    if out.len() > crate::builtins::MAX_STR_LEN {
        return fail(crate::error::LIMIT_ERROR, "JSON output too large");
    }
    Ok(())
}
