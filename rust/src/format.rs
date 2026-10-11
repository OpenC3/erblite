//! Number formatting: `Float#to_s` and `format`/`sprintf`.

use crate::builtins::conv::{str_to_float_strict, str_to_int_strict};
use crate::error::{fail, R, ARGUMENT_ERROR, FLOAT_DOMAIN_ERROR, KEY_ERROR, LIMIT_ERROR, RANGE_ERROR, TYPE_ERROR};
use crate::value::Value;

/// 2**63 as a float.
pub const TWO_POW_63: f64 = i64::MAX as f64;

/// Largest width / precision accepted by `format`.
const MAX_FORMAT_WIDTH: usize = 1 << 20;

/// Shortest round-trip decimal digits `D` and exponent `e` so that `|f| = 0.D × 10^e`.
/// `f` must be finite and non-zero.
///
/// When two shortest candidates are equally close, Ruby's dtoa (mode 0) picks the even
/// digit; the correctly rounded (ties-to-even) string of the same length does the same.
pub fn shortest_digits(f: f64) -> (Vec<u8>, i32) {
    let a = f.abs();
    let split = |s: &str| -> (Vec<u8>, i32) {
        let (mant, exp) = s.split_once('e').unwrap_or((s, "0"));
        let exp: i32 = exp.parse().unwrap_or(0);
        (mant.bytes().filter(|b| b.is_ascii_digit()).collect(), exp + 1)
    };
    let shortest = split(&format!("{:e}", a));
    let n = shortest.0.len();
    if n > 1 {
        let rounded = format!("{:.*e}", n - 1, a);
        if rounded.parse::<f64>().ok() == Some(a) {
            let r = split(&rounded);
            if r.0.len() == n {
                return r;
            }
        }
    }
    shortest
}

/// Ruby 3.4 `Float#to_s`.
pub fn float_to_s(f: f64) -> String {
    if f.is_nan() {
        return "NaN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    if f == 0.0 {
        return if f.is_sign_negative() { "-0.0".into() } else { "0.0".into() };
    }
    let (d, e) = shortest_digits(f);
    let n = d.len() as i32;
    let ds = |a: usize, b: usize| String::from_utf8_lossy(&d[a..b]).into_owned();
    let mut out = String::new();
    if f < 0.0 {
        out.push('-');
    }
    if 0 < e && e <= 16 && n > e {
        out.push_str(&ds(0, e as usize));
        out.push('.');
        out.push_str(&ds(e as usize, d.len()));
    } else if 0 < e && e <= 15 {
        out.push_str(&ds(0, d.len()));
        out.push_str(&"0".repeat((e - n) as usize));
        out.push_str(".0");
    } else if -4 < e && e <= 0 {
        out.push_str("0.");
        out.push_str(&"0".repeat((-e) as usize));
        out.push_str(&ds(0, d.len()));
    } else {
        out.push_str(&ds(0, 1));
        out.push('.');
        if d.len() > 1 {
            out.push_str(&ds(1, d.len()));
        } else {
            out.push('0');
        }
        let x = e - 1;
        out.push('e');
        out.push(if x < 0 { '-' } else { '+' });
        out.push_str(&format!("{:02}", x.abs()));
    }
    out
}

#[derive(Default, Clone, Copy)]
struct Spec {
    minus: bool,
    plus: bool,
    space: bool,
    zero: bool,
    alt: bool,
    width: Option<usize>,
    prec: Option<usize>,
}

enum ArgSource<'a> {
    Positional { args: &'a [Value], next: usize },
}

impl ArgSource<'_> {
    fn next_arg(&mut self) -> R<Value> {
        let ArgSource::Positional { args, next } = self;
        match args.get(*next) {
            Some(v) => {
                *next += 1;
                Ok(v.clone())
            }
            None => fail(ARGUMENT_ERROR, "too few arguments"),
        }
    }

    fn nth(&self, n: usize) -> R<Value> {
        let ArgSource::Positional { args, .. } = self;
        if n == 0 {
            return fail(ARGUMENT_ERROR, "invalid index - 0$");
        }
        match args.get(n - 1) {
            Some(v) => Ok(v.clone()),
            None => fail(ARGUMENT_ERROR, "too few arguments"),
        }
    }

    fn hash_arg(&self) -> R<Value> {
        let ArgSource::Positional { args, .. } = self;
        match args {
            [h @ Value::Hash(_)] => Ok(h.clone()),
            _ => fail(ARGUMENT_ERROR, "one hash required"),
        }
    }
}

fn check_width(n: usize) -> R<usize> {
    if n > MAX_FORMAT_WIDTH {
        fail(LIMIT_ERROR, "format width or precision too large")
    } else {
        Ok(n)
    }
}

/// Converts a `format` argument to an Integer (for `%d`, `%x`, ...).
pub fn arg_to_int(v: &Value) -> R<i64> {
    match v {
        Value::Int(i) => Ok(*i),
        Value::Float(f) => float_to_int(*f),
        Value::Str(s) => str_to_int_strict(s, None),
        Value::Nil => fail(TYPE_ERROR, "can't convert nil into Integer"),
        other => fail(TYPE_ERROR, format!("can't convert {} into Integer", other.type_desc())),
    }
}

/// Truncates a float to an integer, raising like Ruby on NaN, Infinity and overflow.
pub fn float_to_int(f: f64) -> R<i64> {
    if f.is_nan() {
        return fail(FLOAT_DOMAIN_ERROR, "NaN");
    }
    if f.is_infinite() {
        return fail(FLOAT_DOMAIN_ERROR, if f > 0.0 { "Infinity" } else { "-Infinity" });
    }
    let t = f.trunc();
    if !(-TWO_POW_63..TWO_POW_63).contains(&t) {
        return fail(RANGE_ERROR, format!("float {} out of range of integer", float_to_s(f)));
    }
    Ok(t as i64)
}

fn arg_to_float(v: &Value) -> R<f64> {
    match v {
        Value::Int(i) => Ok(*i as f64),
        Value::Float(f) => Ok(*f),
        Value::Str(s) => str_to_float_strict(s),
        Value::Nil => fail(TYPE_ERROR, "can't convert nil into Float"),
        other => fail(TYPE_ERROR, format!("can't convert {} into Float", other.type_desc())),
    }
}

/// Pads `body` (which already contains any sign/prefix) to the spec width with spaces.
fn pad(body: String, spec: &Spec) -> String {
    let len = body.chars().count();
    match spec.width {
        Some(w) if w > len => {
            let fill = " ".repeat(w - len);
            if spec.minus {
                body + &fill
            } else {
                fill + &body
            }
        }
        _ => body,
    }
}

/// Pads a number: `sign_prefix` goes before zero padding, `digits` after.
fn pad_number(sign_prefix: &str, digits: &str, spec: &Spec, allow_zero: bool) -> String {
    let len = sign_prefix.chars().count() + digits.chars().count();
    match spec.width {
        Some(w) if w > len => {
            if spec.minus {
                format!("{}{}{}", sign_prefix, digits, " ".repeat(w - len))
            } else if spec.zero && allow_zero {
                format!("{}{}{}", sign_prefix, "0".repeat(w - len), digits)
            } else {
                format!("{}{}{}", " ".repeat(w - len), sign_prefix, digits)
            }
        }
        _ => format!("{}{}", sign_prefix, digits),
    }
}

fn sign_str(negative: bool, spec: &Spec) -> &'static str {
    if negative {
        "-"
    } else if spec.plus {
        "+"
    } else if spec.space {
        " "
    } else {
        ""
    }
}

fn format_int(v: i64, conv: char, spec: &Spec) -> R<String> {
    let negative = v < 0;
    if negative && !matches!(conv, 'd' | 'i' | 'u') && !spec.plus && !spec.space {
        return fail(
            ARGUMENT_ERROR,
            format!("negative argument for %{} requires the '+' or ' ' flag in ERBLite", conv),
        );
    }
    let mag = (v as i128).unsigned_abs();
    let mut digits = match conv {
        'x' => format!("{:x}", mag),
        'X' => format!("{:X}", mag),
        'o' => format!("{:o}", mag),
        'b' | 'B' => format!("{:b}", mag),
        _ => mag.to_string(),
    };
    if let Some(p) = spec.prec {
        if p == 0 && mag == 0 {
            digits.clear();
        } else if digits.len() < p {
            digits = "0".repeat(p - digits.len()) + &digits;
        }
    }
    let mut prefix = sign_str(negative, spec).to_string();
    if spec.alt && mag != 0 {
        match conv {
            'x' => prefix.push_str("0x"),
            'X' => prefix.push_str("0X"),
            'b' => prefix.push_str("0b"),
            'B' => prefix.push_str("0B"),
            'o' if !digits.starts_with('0') => digits.insert(0, '0'),
            _ => {}
        }
    }
    Ok(pad_number(&prefix, &digits, spec, spec.prec.is_none()))
}

// ----- Ruby's dtoa (missing/dtoa.c) for modes 2 and 3 --------------------------------------
//
// Ruby formats %f/%e/%g with David Gay's dtoa. For up to 14 digits it first tries a fast
// floating-point path; when that path lands within its error bound of a rounding tie it
// rounds half-even on the approximate digits (Ruby's `half` flag) instead of using the exact
// value. Otherwise the result is the correctly rounded exact value, which Rust's formatting
// produces.

const TENS: [f64; 23] = [
    1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16, 1e17, 1e18, 1e19,
    1e20, 1e21, 1e22,
];
const BIGTENS: [f64; 5] = [1e16, 1e32, 1e64, 1e128, 1e256];
const QUICK_MAX: i32 = 14;

/// dtoa's estimate of floor(log10(d)) and whether it may be one too high.
/// The constants are dtoa's own (truncated) values.
#[allow(clippy::approx_constant)]
fn estimate_k(d: f64) -> (i32, bool) {
    let bits = d.to_bits();
    let biased = ((bits >> 52) & 0x7ff) as i32;
    let frac = bits & 0x000F_FFFF_FFFF_FFFF;
    let (d2, i) = if biased != 0 {
        (f64::from_bits(frac | 0x3FF0_0000_0000_0000), biased - 1023)
    } else {
        let lz = frac.leading_zeros() as i32 - 12;
        let shifted = (frac << (lz + 1)) & 0x000F_FFFF_FFFF_FFFF;
        (f64::from_bits(shifted | 0x3FF0_0000_0000_0000), -1022 - (lz + 1))
    };
    let ds = (d2 - 1.5) * 0.289529654602168 + 0.1760912590558 + i as f64 * 0.301029995663981;
    let mut k = ds as i32;
    if ds < 0.0 && ds != k as f64 {
        k -= 1;
    }
    let mut k_check = true;
    if (0..=22).contains(&k) {
        if d < TENS[k as usize] {
            k -= 1;
        }
        k_check = false;
    }
    (k, k_check)
}

fn bump_up(s: &mut Vec<u8>, k: &mut i32) {
    while s.last() == Some(&b'9') {
        if s.len() == 1 {
            *k += 1;
            s[0] = b'1';
            return;
        }
        s.pop();
    }
    if let Some(last) = s.last_mut() {
        *last += 1;
    }
}

/// The fast floating-point path of dtoa. `None` means dtoa falls back to exact arithmetic.
fn dtoa_quick(dv: f64, mode: u8, ndigits: i32) -> Option<(Vec<u8>, i32)> {
    let (mut k, k_check) = estimate_k(dv);
    let (mut ilim, ilim1) = if mode == 2 {
        let n = ndigits.max(1);
        (n, n)
    } else {
        let i = ndigits + k + 1;
        (i, i - 1)
    };
    if !(0..=QUICK_MAX).contains(&ilim) {
        return None;
    }
    let mut d = dv;
    let mut ieps = 2;
    let mut bi = 0;
    if k > 0 {
        let mut ds = TENS[(k & 0xf) as usize];
        let mut j = k >> 4;
        if j & 0x10 != 0 {
            j &= 0x10 - 1;
            d /= BIGTENS[4];
            ieps += 1;
        }
        while j != 0 {
            if j & 1 != 0 {
                ieps += 1;
                ds *= BIGTENS[bi];
            }
            j >>= 1;
            bi += 1;
        }
        d /= ds;
    } else if k < 0 {
        let j1 = -k;
        d *= TENS[(j1 & 0xf) as usize];
        let mut j = j1 >> 4;
        while j != 0 {
            if j & 1 != 0 {
                ieps += 1;
                d *= BIGTENS[bi.min(4)];
            }
            j >>= 1;
            bi += 1;
        }
    }
    if k_check && d < 1.0 && ilim > 0 {
        if ilim1 <= 0 {
            return None;
        }
        ilim = ilim1;
        k -= 1;
        d *= 10.0;
        ieps += 1;
    }
    let e0 = ieps as f64 * d + 7.0;
    let mut eps = f64::from_bits(e0.to_bits().wrapping_sub(52u64 << 52));
    if ilim == 0 {
        d -= 5.0;
        if d > eps {
            return Some((vec![b'1'], k + 2));
        }
        if d < -eps {
            return Some((vec![], -ndigits));
        }
        return None;
    }
    eps *= TENS[(ilim - 1) as usize];
    let mut s: Vec<u8> = Vec::new();
    let mut i = 1;
    loop {
        let l = d as i64;
        d -= l as f64;
        if d == 0.0 {
            ilim = i;
        }
        s.push(b'0' + l.clamp(0, 9) as u8);
        if i == ilim {
            if d > 0.5 + eps {
                bump_up(&mut s, &mut k);
            } else if d >= 0.5 - eps && (s.last().copied().unwrap_or(b'0') - b'0') & 1 == 1 {
                // within the error bound of a tie: Ruby rounds half-even on these digits
                bump_up(&mut s, &mut k);
            }
            while s.len() > 1 && s.last() == Some(&b'0') {
                s.pop();
            }
            return Some((s, k + 1));
        }
        i += 1;
        d *= 10.0;
    }
}

/// Correctly rounded digits (dtoa's exact path).
fn exact_digits(d: f64, mode: u8, ndigits: i32) -> (Vec<u8>, i32) {
    let (mut digits, mut decpt): (Vec<u8>, i32) = if mode == 3 {
        let s = format!("{:.*}", ndigits.max(0) as usize, d);
        let (ip, fp) = s.split_once('.').unwrap_or((s.as_str(), ""));
        let mut all: Vec<u8> = ip.bytes().collect();
        let decpt = all.len() as i32;
        all.extend(fp.bytes());
        (all, decpt)
    } else {
        let s = format!("{:.*e}", (ndigits.max(1) - 1) as usize, d);
        let (m, e) = s.split_once('e').unwrap_or((s.as_str(), "0"));
        let e: i32 = e.parse().unwrap_or(0);
        (m.bytes().filter(|b| b.is_ascii_digit()).collect(), e + 1)
    };
    while digits.first() == Some(&b'0') {
        digits.remove(0);
        decpt -= 1;
    }
    while digits.last() == Some(&b'0') {
        digits.pop();
    }
    if digits.is_empty() {
        decpt = -ndigits;
    }
    (digits, decpt)
}

/// Ruby's `BSD__dtoa(d, mode, ndigits)` for mode 2 (significant digits) and mode 3 (fixed):
/// the digits and decimal point position, value = 0.DIGITS x 10^decpt.
pub fn ruby_dtoa(d: f64, mode: u8, ndigits: i32) -> (Vec<u8>, i32) {
    if d == 0.0 {
        return (vec![b'0'], 1);
    }
    dtoa_quick(d, mode, ndigits).unwrap_or_else(|| exact_digits(d, mode, ndigits))
}

fn digit_at(digits: &[u8], p: i32) -> char {
    if p >= 0 && (p as usize) < digits.len() {
        digits[p as usize] as char
    } else {
        '0'
    }
}

/// Fixed notation with `decimals` digits after the point.
fn fixed_from_digits(digits: &[u8], decpt: i32, decimals: usize, alt: bool) -> String {
    let mut out = String::new();
    if decpt <= 0 {
        out.push('0');
    } else {
        for p in 0..decpt {
            out.push(digit_at(digits, p));
        }
    }
    if decimals > 0 || alt {
        out.push('.');
    }
    for p in 0..decimals as i32 {
        out.push(digit_at(digits, decpt + p));
    }
    out
}

fn exp_suffix(e: i32, upper: bool) -> String {
    format!("{}{}{:02}", if upper { 'E' } else { 'e' }, if e < 0 { '-' } else { '+' }, e.abs())
}

/// `%e` body: `prec` digits after the point.
fn exp_from_digits(digits: &[u8], decpt: i32, prec: usize, alt: bool, upper: bool) -> String {
    let mut out = String::new();
    out.push(digit_at(digits, 0));
    if prec > 0 || alt {
        out.push('.');
    }
    for p in 1..=prec as i32 {
        out.push(digit_at(digits, p));
    }
    let e = if digits.is_empty() || digits == b"0" { 0 } else { decpt - 1 };
    out.push_str(&exp_suffix(e, upper));
    out
}

fn format_float(f: f64, conv: char, spec: &Spec) -> String {
    let negative = f.is_sign_negative() && !f.is_nan();
    if !f.is_finite() {
        let body = if f.is_nan() { "NaN" } else { "Inf" };
        return pad_number(sign_str(negative, spec), body, spec, false);
    }
    let a = f.abs();
    let prec = spec.prec.unwrap_or(6);
    let body = match conv {
        'f' => {
            let (digits, decpt) = ruby_dtoa(a, 3, prec as i32);
            fixed_from_digits(&digits, decpt, prec, spec.alt)
        }
        'e' | 'E' => {
            let (digits, decpt) = ruby_dtoa(a, 2, prec as i32 + 1);
            exp_from_digits(&digits, decpt, prec, spec.alt, conv == 'E')
        }
        _ => {
            let p = if prec == 0 { 1 } else { prec };
            let (digits, decpt) = ruby_dtoa(a, 2, p as i32);
            let expt = if a == 0.0 { 1 } else { decpt };
            if expt <= -4 || expt > p as i32 {
                let shown = if spec.alt { p - 1 } else { digits.len().saturating_sub(1) };
                exp_from_digits(&digits, expt, shown, spec.alt, conv == 'G')
            } else {
                let decimals = if spec.alt {
                    (p as i32 - expt).max(0) as usize
                } else {
                    (digits.len() as i32 - expt).max(0) as usize
                };
                fixed_from_digits(&digits, expt, decimals, spec.alt)
            }
        }
    };
    pad_number(sign_str(negative, spec), &body, spec, true)
}

fn truncate_chars(s: String, prec: Option<usize>) -> String {
    match prec {
        Some(p) => s.chars().take(p).collect(),
        None => s,
    }
}

/// Ruby `format(fmt, *args)`.
pub fn sprintf(fmt: &str, args: &[Value]) -> R<String> {
    let chars: Vec<char> = fmt.chars().collect();
    let mut src = ArgSource::Positional { args, next: 0 };
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c != '%' {
            out.push(c);
            i += 1;
            continue;
        }
        i += 1;
        if i >= chars.len() {
            return fail(ARGUMENT_ERROR, "incomplete format specifier; use %% (double %) instead");
        }
        if chars[i] == '%' {
            out.push('%');
            i += 1;
            continue;
        }
        let mut spec = Spec::default();
        let mut named: Option<Value> = None;
        let mut explicit: Option<Value> = None;
        let mut brace_done = false;
        loop {
            if i >= chars.len() {
                return fail(ARGUMENT_ERROR, "incomplete format specifier; use %% (double %) instead");
            }
            match chars[i] {
                '-' => spec.minus = true,
                '+' => spec.plus = true,
                ' ' => spec.space = true,
                '0' => spec.zero = true,
                '#' => spec.alt = true,
                '<' | '{' => {
                    let close = if chars[i] == '<' { '>' } else { '}' };
                    let start = i + 1;
                    let mut j = start;
                    while j < chars.len() && chars[j] != close {
                        j += 1;
                    }
                    if j >= chars.len() {
                        return fail(ARGUMENT_ERROR, "malformed name - unmatched parenthesis");
                    }
                    let name: String = chars[start..j].iter().collect();
                    let h = src.hash_arg()?;
                    let val = match &h {
                        Value::Hash(hc) => {
                            let hb = hc.borrow();
                            match hb.get(&Value::sym(&name)) {
                                Some(v) => v,
                                None => {
                                    if hb.default.is_nil() {
                                        return fail(KEY_ERROR, format!("key{}{}{} not found", chars[i], name, close));
                                    }
                                    hb.default.clone()
                                }
                            }
                        }
                        _ => Value::Nil,
                    };
                    i = j;
                    if close == '}' {
                        let s = pad(truncate_chars(val.to_s()?, spec.prec), &spec);
                        out.push_str(&s);
                        i += 1;
                        brace_done = true;
                        break;
                    }
                    named = Some(val);
                }
                '*' => {
                    let w = arg_to_int(&src.next_arg()?)?;
                    if w < 0 {
                        spec.minus = true;
                    }
                    spec.width = Some(check_width(w.unsigned_abs() as usize)?);
                }
                '1'..='9' => {
                    let mut n: usize = 0;
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        n = n.saturating_mul(10).saturating_add(chars[i] as usize - '0' as usize);
                        i += 1;
                    }
                    if i < chars.len() && chars[i] == '$' {
                        explicit = Some(src.nth(n)?);
                    } else {
                        spec.width = Some(check_width(n)?);
                        continue;
                    }
                }
                '.' => {
                    i += 1;
                    if i < chars.len() && chars[i] == '*' {
                        let p = arg_to_int(&src.next_arg()?)?;
                        spec.prec = if p < 0 { None } else { Some(check_width(p as usize)?) };
                        i += 1;
                    } else {
                        let mut n: usize = 0;
                        while i < chars.len() && chars[i].is_ascii_digit() {
                            n = n.saturating_mul(10).saturating_add(chars[i] as usize - '0' as usize);
                            i += 1;
                        }
                        spec.prec = Some(check_width(n)?);
                    }
                    continue;
                }
                _ => break,
            }
            i += 1;
        }
        if brace_done {
            continue;
        }
        if i >= chars.len() {
            return fail(ARGUMENT_ERROR, "incomplete format specifier; use %% (double %) instead");
        }
        let conv = chars[i];
        i += 1;
        let mut take = |src: &mut ArgSource| -> R<Value> {
            if let Some(v) = named.take() {
                return Ok(v);
            }
            if let Some(v) = explicit.take() {
                return Ok(v);
            }
            src.next_arg()
        };
        let piece = match conv {
            'd' | 'i' | 'u' | 'x' | 'X' | 'o' | 'b' | 'B' => {
                let v = arg_to_int(&take(&mut src)?)?;
                format_int(v, conv, &spec)?
            }
            'f' | 'e' | 'E' | 'g' | 'G' => {
                let v = arg_to_float(&take(&mut src)?)?;
                format_float(v, conv, &spec)
            }
            's' => pad(truncate_chars(take(&mut src)?.to_s()?, spec.prec), &spec),
            'p' => pad(truncate_chars(take(&mut src)?.inspect()?, spec.prec), &spec),
            'c' => {
                let v = take(&mut src)?;
                let ch = match &v {
                    Value::Str(s) => s.chars().next().map(String::from).unwrap_or_default(),
                    other => {
                        let n = arg_to_int(other)?;
                        match u32::try_from(n).ok().and_then(char::from_u32) {
                            Some(c) => c.to_string(),
                            None if n < 0 => return fail(ARGUMENT_ERROR, "invalid character"),
                            None => return fail(RANGE_ERROR, format!("{} out of char range", n)),
                        }
                    }
                };
                pad(ch, &spec)
            }
            '\n' | '\0' => {
                out.push('%');
                out.push(conv);
                continue;
            }
            other => {
                return fail(ARGUMENT_ERROR, format!("malformed format string - %{}", other));
            }
        };
        out.push_str(&piece);
    }
    Ok(out)
}
