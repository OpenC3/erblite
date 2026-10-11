// Copyright 2026 OpenC3, Inc.
// All Rights Reserved.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
// See LICENSE.md for more details.

//! String to number conversions: lenient (`to_i`, `to_f`) and strict (`Integer()`, `Float()`).

use crate::error::{fail, ARGUMENT_ERROR, R, RANGE_ERROR};

const WS: &[char] = &[' ', '\t', '\n', '\x0b', '\x0c', '\r'];

fn overflow<T>() -> R<T> {
    fail(RANGE_ERROR, "integer out of 64-bit range")
}

/// Reads digits of `radix` with single underscores between digits. Returns (digits, rest).
fn take_digits(s: &str, radix: u32) -> (String, &str) {
    let mut digits = String::new();
    let mut end = 0;
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    let mut i = 0;
    while i < chars.len() {
        let (pos, c) = chars[i];
        if c.is_digit(radix) {
            digits.push(c);
            end = pos + c.len_utf8();
        } else if c == '_' && !digits.is_empty() && i + 1 < chars.len() && chars[i + 1].1.is_digit(radix) {
            // single underscore between digits
        } else {
            break;
        }
        i += 1;
    }
    (digits, &s[end..])
}

fn digits_to_i64(digits: &str, radix: u32, negative: bool) -> R<i64> {
    let mut v: i128 = 0;
    for c in digits.chars() {
        v = v * radix as i128 + c.to_digit(radix).unwrap_or(0) as i128;
        if v > (i64::MAX as i128) + 1 {
            return overflow();
        }
    }
    if negative {
        v = -v;
    }
    i64::try_from(v).or_else(|_| overflow())
}

/// `String#to_i(base)`.
pub fn str_to_i(s: &str, base: u32) -> R<i64> {
    let mut t = s.trim_start_matches(WS);
    let mut negative = false;
    if let Some(r) = t.strip_prefix('-') {
        negative = true;
        t = r;
    } else if let Some(r) = t.strip_prefix('+') {
        t = r;
    }
    let lower = t.to_ascii_lowercase();
    let prefix = match base {
        16 => "0x",
        2 => "0b",
        8 => "0o",
        10 => "0d",
        _ => "",
    };
    if !prefix.is_empty() && lower.starts_with(prefix) {
        let rest = &t[2..];
        if rest.chars().next().is_some_and(|c| c.is_digit(base)) {
            t = rest;
        }
    }
    let (digits, _) = take_digits(t, base);
    if digits.is_empty() {
        return Ok(0);
    }
    digits_to_i64(&digits, base, negative)
}

/// `String#oct`: base 8, but honors `0x`, `0b`, `0o` prefixes.
pub fn str_oct(s: &str) -> R<i64> {
    let t = s.trim_start_matches(WS);
    let (sign, body) = match t.strip_prefix('-') {
        Some(r) => ("-", r),
        None => ("", t.strip_prefix('+').unwrap_or(t)),
    };
    let lower = body.to_ascii_lowercase();
    let base = if lower.starts_with("0x") {
        16
    } else if lower.starts_with("0b") {
        2
    } else {
        8
    };
    str_to_i(&format!("{}{}", sign, body), base)
}

/// `String#to_f`.
pub fn str_to_f(s: &str) -> f64 {
    let t = s.trim_start_matches(WS);
    let mut out = String::new();
    let mut rest = t;
    if let Some(r) = rest.strip_prefix('-') {
        out.push('-');
        rest = r;
    } else if let Some(r) = rest.strip_prefix('+') {
        rest = r;
    }
    let (int_digits, r) = take_digits(rest, 10);
    rest = r;
    out.push_str(&int_digits);
    let mut has_digits = !int_digits.is_empty();
    if let Some(r) = rest.strip_prefix('.') {
        let (frac, r2) = take_digits(r, 10);
        if !frac.is_empty() {
            out.push('.');
            out.push_str(&frac);
            rest = r2;
            has_digits = true;
        }
    }
    if !has_digits {
        return 0.0;
    }
    if let Some(r) = rest.strip_prefix(['e', 'E']) {
        let (sign, r) = match r.strip_prefix('-') {
            Some(x) => ("-", x),
            None => ("", r.strip_prefix('+').unwrap_or(r)),
        };
        let (exp, _) = take_digits(r, 10);
        if !exp.is_empty() {
            out.push('e');
            out.push_str(sign);
            out.push_str(&exp);
        }
    }
    out.parse::<f64>().unwrap_or(0.0)
}

/// Strict `Integer(str, base)`.
pub fn str_to_int_strict(s: &str, base: Option<i64>) -> R<i64> {
    let invalid = || fail(ARGUMENT_ERROR, format!("invalid value for Integer(): {:?}", s));
    let t = s.trim_matches(WS);
    let (negative, body) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let lower = body.to_ascii_lowercase();
    let (radix, digits_part) = {
        let pref = |p: &str| lower.starts_with(p) && body.len() > 2;
        let b = base.unwrap_or(0);
        if pref("0x") && (b == 0 || b == 16) {
            (16, &body[2..])
        } else if pref("0b") && (b == 0 || b == 2) {
            (2, &body[2..])
        } else if pref("0o") && (b == 0 || b == 8) {
            (8, &body[2..])
        } else if pref("0d") && (b == 0 || b == 10) {
            (10, &body[2..])
        } else if b == 0 && body.len() > 1 && body.starts_with('0') {
            let rest = &body[1..];
            (8, rest.strip_prefix('_').unwrap_or(rest))
        } else if b == 0 {
            (10, body)
        } else if (2..=36).contains(&b) {
            (b as u32, body)
        } else {
            return fail(ARGUMENT_ERROR, format!("invalid radix {}", b));
        }
    };
    let (digits, rest) = take_digits(digits_part, radix);
    if digits.is_empty() || !rest.is_empty() {
        return invalid();
    }
    digits_to_i64(&digits, radix, negative)
}

/// Strict `Float(str)`.
pub fn str_to_float_strict(s: &str) -> R<f64> {
    let invalid = || fail(ARGUMENT_ERROR, format!("invalid value for Float(): {:?}", s));
    let t = s.trim_matches(WS);
    let (sign, body) = match t.strip_prefix('-') {
        Some(r) => ("-", r),
        None => ("", t.strip_prefix('+').unwrap_or(t)),
    };
    let lower = body.to_ascii_lowercase();
    if lower.starts_with("0x") {
        let (digits, rest) = take_digits(&body[2..], 16);
        if digits.is_empty() || !rest.is_empty() {
            return invalid();
        }
        let v = digits_to_i64(&digits, 16, sign == "-")?;
        return Ok(v as f64);
    }
    let mut out = String::from(sign);
    let (int_digits, mut rest) = take_digits(body, 10);
    out.push_str(&int_digits);
    let mut has = !int_digits.is_empty();
    if let Some(r) = rest.strip_prefix('.') {
        if r.starts_with('_') {
            return invalid();
        }
        let (frac, r2) = take_digits(r, 10);
        if !frac.is_empty() {
            out.push('.');
            out.push_str(&frac);
            has = true;
        } else if !has {
            return invalid();
        }
        rest = r2;
    }
    if !has {
        return invalid();
    }
    if let Some(r) = rest.strip_prefix(['e', 'E']) {
        let (esign, r) = match r.strip_prefix('-') {
            Some(x) => ("-", x),
            None => ("", r.strip_prefix('+').unwrap_or(r)),
        };
        let (exp, r2) = take_digits(r, 10);
        if exp.is_empty() {
            return invalid();
        }
        out.push('e');
        out.push_str(esign);
        out.push_str(&exp);
        rest = r2;
    }
    if !rest.is_empty() {
        return invalid();
    }
    out.parse::<f64>().or_else(|_| invalid())
}
