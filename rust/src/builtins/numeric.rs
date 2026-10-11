// Copyright 2026 OpenC3, Inc.
// All Rights Reserved.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
// See LICENSE.md for more details.

//! Integer (§8.3) and Float (§8.4) methods.

use super::{argc, block_or_enum, object, to_int};
use crate::error::{fail, rt_err, Flow, ARGUMENT_ERROR, MATH_DOMAIN_ERROR, R, RANGE_ERROR, TYPE_ERROR, ZERO_DIV_ERROR};
use crate::format::{float_to_int, float_to_s};
use crate::interp::{BlockArg, Interp};
use crate::value::Value;
use std::cmp::Ordering;

pub fn int_overflow<T>() -> R<T> {
    fail(RANGE_ERROR, "integer overflow (ERBLite integers are 64-bit)")
}

fn coerce_err(other: &Value, into: &str) -> Flow {
    rt_err(
        TYPE_ERROR,
        format!("{} can't be coerced into {}", other.type_desc(), into),
    )
}

/// `comparison of X with Y failed`.
pub fn comparison_failed(a: &Value, b: &Value) -> Flow {
    let bd = match b {
        Value::Nil | Value::Bool(_) => b.type_desc().to_string(),
        Value::Int(_) | Value::Float(_) => b.inspect().unwrap_or_default(),
        _ => b.class_name().to_string(),
    };
    rt_err(
        ARGUMENT_ERROR,
        format!("comparison of {} with {} failed", a.class_name(), bd),
    )
}

/// Compares two values for `<`, `sort`, `min`, ...; raises when incomparable.
pub fn compare_or_fail(a: &Value, b: &Value) -> R<Ordering> {
    a.compare(b).ok_or_else(|| comparison_failed(a, b))
}

pub fn floor_div(a: i64, b: i64) -> R<i64> {
    if b == 0 {
        return fail(ZERO_DIV_ERROR, "divided by 0");
    }
    let Some(q) = a.checked_div(b) else {
        return int_overflow();
    };
    if (a % b != 0) && ((a < 0) != (b < 0)) {
        Ok(q - 1)
    } else {
        Ok(q)
    }
}

pub fn floor_mod(a: i64, b: i64) -> R<i64> {
    if b == 0 {
        return fail(ZERO_DIV_ERROR, "divided by 0");
    }
    let r = a.checked_rem(b).unwrap_or(0);
    if r != 0 && ((r < 0) != (b < 0)) {
        Ok(r + b)
    } else {
        Ok(r)
    }
}

/// Ruby `flodivmod`.
fn flo_divmod(x: f64, y: f64) -> R<(f64, f64)> {
    if y.is_nan() {
        return Ok((y, y));
    }
    if y == 0.0 {
        return fail(ZERO_DIV_ERROR, "divided by 0");
    }
    let mut m = if x == 0.0 || (y.is_infinite() && !x.is_infinite()) {
        x
    } else {
        x % y
    };
    let mut d = if x.is_infinite() && !y.is_infinite() {
        x
    } else {
        ((x - m) / y).round()
    };
    if y * m < 0.0 {
        m += y;
        d -= 1.0;
    }
    Ok((d, m))
}

/// Ruby `Float#%` (`flodivmod`): a zero divisor raises ZeroDivisionError.
pub fn float_mod(x: f64, y: f64) -> R<f64> {
    Ok(flo_divmod(x, y)?.1)
}

fn to_f(v: &Value, into: &str) -> R<f64> {
    match v {
        Value::Int(i) => Ok(*i as f64),
        Value::Float(f) => Ok(*f),
        other => Err(coerce_err(other, into)),
    }
}

fn float_pow(a: f64, e: f64) -> R<Value> {
    if a < 0.0 && e.is_finite() && e.fract() != 0.0 {
        return fail(
            RANGE_ERROR,
            "negative number to a fractional power gives a Complex result (not supported)",
        );
    }
    Ok(Value::Float(a.powf(e)))
}

fn int_pow(a: i64, e: i64) -> R<Value> {
    if e < 0 {
        return fail(RANGE_ERROR, "negative exponent (ERBLite has no Rational)");
    }
    match u32::try_from(e) {
        Ok(e32) => match a.checked_pow(e32) {
            Some(v) => Ok(Value::Int(v)),
            None => int_overflow(),
        },
        Err(_) => match a {
            0 | 1 => Ok(Value::Int(a)),
            -1 => Ok(Value::Int(if e % 2 == 0 { 1 } else { -1 })),
            _ => int_overflow(),
        },
    }
}

fn int_to_s(i: i64, base: u32) -> String {
    let mut mag = (i as i128).unsigned_abs();
    if mag == 0 {
        return "0".into();
    }
    let mut digits = Vec::new();
    while mag > 0 {
        digits.push(std::char::from_digit((mag % base as u128) as u32, base).unwrap_or('0'));
        mag /= base as u128;
    }
    if i < 0 {
        digits.push('-');
    }
    digits.iter().rev().collect()
}

fn pow10(n: u32) -> Option<i64> {
    10i64.checked_pow(n)
}

/// Rounds an integer to a multiple of 10**(-nd) for nd < 0. `mode`: 'r' round half up, 'f' floor,
/// 'c' ceil, 't' truncate.
pub fn int_round_digits(x: i64, nd: i64, mode: char) -> R<i64> {
    if nd >= 0 {
        return Ok(x);
    }
    let Some(p) = u32::try_from(-nd).ok().and_then(pow10) else {
        // |x| < 10**19 <= p: result is 0 except floor of negatives / ceil of positives
        return match mode {
            'f' if x < 0 => int_overflow(),
            'c' if x > 0 => int_overflow(),
            _ => Ok(0),
        };
    };
    let r = x % p; // truncated remainder, same sign as x
    let base = x - r;
    let res = match mode {
        'r' => {
            if (r.unsigned_abs()) * 2 >= p as u64 {
                if x < 0 {
                    base.checked_sub(p)
                } else {
                    base.checked_add(p)
                }
            } else {
                Some(base)
            }
        }
        'f' => {
            if r < 0 {
                base.checked_sub(p)
            } else {
                Some(base)
            }
        }
        'c' => {
            if r > 0 {
                base.checked_add(p)
            } else {
                Some(base)
            }
        }
        _ => Some(base),
    };
    res.map_or_else(int_overflow, Ok)
}

fn frexp_exp(x: f64) -> i32 {
    if x == 0.0 || !x.is_finite() {
        return 0;
    }
    let bits = x.to_bits();
    let e = ((bits >> 52) & 0x7ff) as i32;
    if e == 0 {
        // subnormal: scale up
        let y = x * 2f64.powi(64);
        let e2 = ((y.to_bits() >> 52) & 0x7ff) as i32;
        return e2 - 1022 - 64;
    }
    e - 1022
}

fn float_round_overflow(ndigits: i64, binexp: i32) -> bool {
    let b = binexp as i64;
    ndigits >= 17 - if b > 0 { b / 4 } else { b / 3 - 1 }
}

fn float_round_underflow(ndigits: i64, binexp: i32) -> bool {
    let b = binexp as i64;
    ndigits < -(if b > 0 { b / 3 + 1 } else { b / 4 })
}

fn round_half_up(x: f64, s: f64) -> f64 {
    let xs = x * s;
    let mut f = xs.round();
    if s == 1.0 {
        return f;
    }
    if x > 0.0 {
        if (f + 0.5) / s <= x {
            f += 1.0;
        }
    } else if (f - 0.5) / s >= x {
        f -= 1.0;
    }
    f
}

/// Ruby `Float#round(ndigits)` for ndigits > 0.
fn float_round_pos(x: f64, nd: i64) -> f64 {
    if x == 0.0 {
        return x;
    }
    let binexp = frexp_exp(x);
    if float_round_overflow(nd, binexp) {
        return x;
    }
    if float_round_underflow(nd, binexp) {
        return 0.0;
    }
    if nd > 14 {
        // Ruby uses exact rational rounding here; decimal formatting is correctly rounded
        // from the exact binary value, which matches except for exact decimal ties.
        let s = format!("{:.*}", nd as usize, x);
        return s.parse().unwrap_or(x);
    }
    let s = 10f64.powi(nd as i32);
    round_half_up(x, s) / s
}

fn float_floor_ceil(x: f64, nd: i64, ceil: bool) -> R<Value> {
    if x == 0.0 {
        return Ok(if nd > 0 { Value::Float(x) } else { Value::Int(0) });
    }
    if nd > 0 {
        let binexp = frexp_exp(x);
        if float_round_overflow(nd, binexp) {
            return Ok(Value::Float(x));
        }
        if (if ceil { x < 0.0 } else { x > 0.0 }) && float_round_underflow(nd, binexp) {
            return Ok(Value::Float(0.0));
        }
        let f = 10f64.powi(nd.min(400) as i32);
        if ceil {
            return Ok(Value::Float((x * f).ceil() / f));
        }
        let mul = (x * f).floor();
        let mut res = (mul + 1.0) / f;
        if res > x {
            res = mul / f;
        }
        return Ok(Value::Float(res));
    }
    let v = float_to_int(if ceil { x.ceil() } else { x.floor() })?;
    Ok(Value::Int(int_round_digits(v, nd, if ceil { 'c' } else { 'f' })?))
}

fn ndigits_arg(args: &[Value]) -> R<i64> {
    argc(args, 0, 1)?;
    match args.first() {
        None => Ok(0),
        Some(v) => to_int(v),
    }
}

/// Ruby's float step count (`ruby_float_step_size`).
pub fn float_step_size(beg: f64, end: f64, unit: f64, excl: bool) -> f64 {
    if unit == 0.0 {
        return f64::INFINITY;
    }
    if unit.is_infinite() {
        return if unit > 0.0 {
            (beg <= end) as i32 as f64
        } else {
            (beg >= end) as i32 as f64
        };
    }
    let n = (end - beg) / unit;
    let err = ((beg.abs() + end.abs() + (end - beg).abs()) / unit.abs() * f64::EPSILON).min(0.5);
    if excl {
        if n <= 0.0 {
            return 0.0;
        }
        let mut n = if n < 1.0 { 0.0 } else { (n - err).floor() };
        let d = (n + 1.0) * unit + beg;
        if beg < end {
            if d < end {
                n += 1.0;
            }
        } else if beg > end && d > end {
            n += 1.0;
        }
        return n + 1.0;
    }
    if n < 0.0 {
        return 0.0;
    }
    (n + err).floor() + 1.0
}

/// Yields Ruby's float step sequence.
pub fn float_step(it: &mut Interp, b: &BlockArg, beg: f64, end: f64, unit: f64, excl: bool) -> R<()> {
    let n = float_step_size(beg, end, unit, excl);
    if unit.is_infinite() {
        if n > 0.0 {
            it.yield1(b, Value::Float(beg))?;
        }
        return Ok(());
    }
    if unit == 0.0 {
        loop {
            it.yield1(b, Value::Float(beg))?;
        }
    }
    let mut i = 0.0;
    while i < n {
        let mut d = i * unit + beg;
        if if unit >= 0.0 { end < d } else { d < end } {
            d = end;
        }
        it.yield1(b, Value::Float(d))?;
        i += 1.0;
    }
    Ok(())
}

pub fn between(recv: &Value, args: &[Value]) -> R<Value> {
    argc(args, 2, 2)?;
    let lo = compare_or_fail(recv, &args[0])?;
    let hi = compare_or_fail(recv, &args[1])?;
    Ok(Value::Bool(lo != Ordering::Less && hi != Ordering::Greater))
}

pub fn clamp(recv: &Value, args: &[Value]) -> R<Value> {
    argc(args, 2, 2)?;
    if compare_or_fail(&args[0], &args[1])? == Ordering::Greater {
        return fail(
            ARGUMENT_ERROR,
            "min argument must be less than or equal to max argument",
        );
    }
    if compare_or_fail(recv, &args[0])? == Ordering::Less {
        return Ok(args[0].clone());
    }
    if compare_or_fail(recv, &args[1])? == Ordering::Greater {
        return Ok(args[1].clone());
    }
    Ok(recv.clone())
}

fn cmp_op(recv: &Value, name: &str, args: &[Value]) -> R<Value> {
    argc(args, 1, 1)?;
    if name == "<=>" {
        return Ok(match recv.compare(&args[0]) {
            Some(o) => Value::Int(o as i64),
            None => Value::Nil,
        });
    }
    let o = compare_or_fail(recv, &args[0])?;
    Ok(Value::Bool(match name {
        "<" => o == Ordering::Less,
        "<=" => o != Ordering::Greater,
        ">" => o == Ordering::Greater,
        _ => o != Ordering::Less,
    }))
}

pub fn call_int(
    it: &mut Interp,
    i: i64,
    recv: &Value,
    name: &str,
    args: Vec<Value>,
    blk: Option<&BlockArg>,
) -> R<Value> {
    match name {
        "+" | "-" | "*" => {
            argc(&args, 1, 1)?;
            match &args[0] {
                Value::Int(j) => {
                    let r = match name {
                        "+" => i.checked_add(*j),
                        "-" => i.checked_sub(*j),
                        _ => i.checked_mul(*j),
                    };
                    r.map(Value::Int).map_or_else(int_overflow, Ok)
                }
                Value::Float(f) => {
                    let a = i as f64;
                    Ok(Value::Float(match name {
                        "+" => a + f,
                        "-" => a - f,
                        _ => a * f,
                    }))
                }
                other => Err(coerce_err(other, "Integer")),
            }
        }
        "/" => {
            argc(&args, 1, 1)?;
            match &args[0] {
                Value::Int(j) => Ok(Value::Int(floor_div(i, *j)?)),
                Value::Float(f) => Ok(Value::Float(i as f64 / f)),
                other => Err(coerce_err(other, "Integer")),
            }
        }
        "%" => {
            argc(&args, 1, 1)?;
            match &args[0] {
                Value::Int(j) => Ok(Value::Int(floor_mod(i, *j)?)),
                Value::Float(f) => Ok(Value::Float(float_mod(i as f64, *f)?)),
                other => Err(coerce_err(other, "Integer")),
            }
        }
        "**" => {
            argc(&args, 1, 1)?;
            match &args[0] {
                Value::Int(e) => int_pow(i, *e),
                Value::Float(e) => float_pow(i as f64, *e),
                other => Err(coerce_err(other, "Integer")),
            }
        }
        "-@" => {
            argc(&args, 0, 0)?;
            i.checked_neg().map(Value::Int).map_or_else(int_overflow, Ok)
        }
        "~" => {
            argc(&args, 0, 0)?;
            Ok(Value::Int(!i))
        }
        "&" | "|" | "^" => {
            argc(&args, 1, 1)?;
            let Value::Int(j) = &args[0] else {
                return Err(coerce_err(&args[0], "Integer"));
            };
            Ok(Value::Int(match name {
                "&" => i & j,
                "|" => i | j,
                _ => i ^ j,
            }))
        }
        "<<" | ">>" => {
            argc(&args, 1, 1)?;
            let s = to_int(&args[0])?;
            let left = (name == "<<") == (s >= 0);
            let n = s.unsigned_abs();
            if left {
                if i == 0 {
                    return Ok(Value::Int(0));
                }
                if n >= 64 {
                    return int_overflow();
                }
                let r = i << n;
                if (r >> n) != i {
                    return int_overflow();
                }
                Ok(Value::Int(r))
            } else if n >= 64 {
                Ok(Value::Int(if i < 0 { -1 } else { 0 }))
            } else {
                Ok(Value::Int(i >> n))
            }
        }
        "<=>" | "<" | "<=" | ">" | ">=" => cmp_op(recv, name, &args),
        "abs" => {
            argc(&args, 0, 0)?;
            i.checked_abs().map(Value::Int).map_or_else(int_overflow, Ok)
        }
        "succ" | "next" => {
            argc(&args, 0, 0)?;
            i.checked_add(1).map(Value::Int).map_or_else(int_overflow, Ok)
        }
        "pred" => {
            argc(&args, 0, 0)?;
            i.checked_sub(1).map(Value::Int).map_or_else(int_overflow, Ok)
        }
        "to_i" | "ord" => {
            argc(&args, 0, 0)?;
            Ok(recv.clone())
        }
        "to_f" => {
            argc(&args, 0, 0)?;
            Ok(Value::Float(i as f64))
        }
        "to_s" => {
            argc(&args, 0, 1)?;
            let base = match args.first() {
                Some(b) => to_int(b)?,
                None => 10,
            };
            if !(2..=36).contains(&base) {
                return fail(ARGUMENT_ERROR, format!("invalid radix {}", base));
            }
            Ok(Value::string(int_to_s(i, base as u32)))
        }
        "chr" => {
            argc(&args, 0, 0)?;
            match u8::try_from(i) {
                Ok(b) => Ok(Value::string((b as char).to_string())),
                Err(_) => fail(RANGE_ERROR, format!("{} out of char range", i)),
            }
        }
        "times" => {
            argc(&args, 0, 0)?;
            let b = block_or_enum!(blk, recv, name, args);
            let mut k = 0;
            while k < i {
                it.yield1(b, Value::Int(k))?;
                k += 1;
            }
            Ok(recv.clone())
        }
        "upto" | "downto" => {
            argc(&args, 1, 1)?;
            let b = block_or_enum!(blk, recv, name, args);
            let up = name == "upto";
            match &args[0] {
                Value::Int(lim) => {
                    let mut k = i;
                    while if up { k <= *lim } else { k >= *lim } {
                        it.yield1(b, Value::Int(k))?;
                        match if up { k.checked_add(1) } else { k.checked_sub(1) } {
                            Some(n) => k = n,
                            None => break,
                        }
                    }
                }
                Value::Float(lim) => {
                    let mut k = i;
                    while if up { (k as f64) <= *lim } else { (k as f64) >= *lim } {
                        it.yield1(b, Value::Int(k))?;
                        match if up { k.checked_add(1) } else { k.checked_sub(1) } {
                            Some(n) => k = n,
                            None => break,
                        }
                    }
                }
                other => return Err(comparison_failed(recv, other)),
            }
            Ok(recv.clone())
        }
        "step" => {
            argc(&args, 1, 2)?;
            let b = block_or_enum!(blk, recv, name, args);
            let step = args.get(1).cloned().unwrap_or(Value::Int(1));
            num_step(it, b, recv, &args[0], &step)?;
            Ok(recv.clone())
        }
        "even?" | "odd?" | "zero?" | "positive?" | "negative?" => {
            argc(&args, 0, 0)?;
            Ok(Value::Bool(match name {
                "even?" => i % 2 == 0,
                "odd?" => i % 2 != 0,
                "zero?" => i == 0,
                "positive?" => i > 0,
                _ => i < 0,
            }))
        }
        "between?" => between(recv, &args),
        "clamp" => clamp(recv, &args),
        "divmod" => {
            argc(&args, 1, 1)?;
            match &args[0] {
                Value::Int(j) => Ok(Value::array(vec![
                    Value::Int(floor_div(i, *j)?),
                    Value::Int(floor_mod(i, *j)?),
                ])),
                Value::Float(f) => {
                    let (d, m) = flo_divmod(i as f64, *f)?;
                    Ok(Value::array(vec![Value::Int(float_to_int(d)?), Value::Float(m)]))
                }
                other => Err(coerce_err(other, "Integer")),
            }
        }
        "fdiv" => {
            argc(&args, 1, 1)?;
            Ok(Value::Float(i as f64 / to_f(&args[0], "Integer")?))
        }
        "gcd" | "lcm" => {
            argc(&args, 1, 1)?;
            let Value::Int(j) = &args[0] else {
                return fail(TYPE_ERROR, "not an integer");
            };
            let (mut a, mut b2) = ((i as i128).abs(), (*j as i128).abs());
            let (x, y) = (a, b2);
            while b2 != 0 {
                let t = a % b2;
                a = b2;
                b2 = t;
            }
            let r = if name == "gcd" {
                a
            } else if x == 0 || y == 0 {
                0
            } else {
                x / a * y
            };
            i64::try_from(r).map(Value::Int).or_else(|_| int_overflow())
        }
        "digits" => {
            argc(&args, 0, 1)?;
            let base = match args.first() {
                Some(b) => to_int(b)?,
                None => 10,
            };
            if base < 2 {
                return fail(ARGUMENT_ERROR, format!("invalid radix {}", base));
            }
            if i < 0 {
                return fail(MATH_DOMAIN_ERROR, "out of domain");
            }
            let mut v = i;
            let mut out = vec![];
            loop {
                out.push(Value::Int(v % base));
                v /= base;
                if v == 0 {
                    break;
                }
            }
            Ok(Value::array(out))
        }
        "bit_length" => {
            argc(&args, 0, 0)?;
            Ok(Value::Int(if i >= 0 {
                64 - i.leading_zeros() as i64
            } else {
                64 - i.leading_ones() as i64
            }))
        }
        "floor" | "ceil" | "truncate" | "round" => {
            let nd = ndigits_arg(&args)?;
            let mode = match name {
                "floor" => 'f',
                "ceil" => 'c',
                "truncate" => 't',
                _ => 'r',
            };
            Ok(Value::Int(int_round_digits(i, nd, mode)?))
        }
        _ => object::call(it, recv, name, args, blk),
    }
}

/// `Numeric#step(limit, step)`.
fn num_step(it: &mut Interp, b: &BlockArg, recv: &Value, limit: &Value, step: &Value) -> R<()> {
    let any_float =
        matches!(recv, Value::Float(_)) || matches!(limit, Value::Float(_)) || matches!(step, Value::Float(_));
    if any_float {
        let beg = to_f(recv, "Float")?;
        let end = match limit {
            Value::Nil => {
                if to_f(step, "Float")? < 0.0 {
                    f64::NEG_INFINITY
                } else {
                    f64::INFINITY
                }
            }
            v => to_f(v, "Float")?,
        };
        let unit = to_f(step, "Float")?;
        if unit == 0.0 {
            return fail(ARGUMENT_ERROR, "step can't be 0");
        }
        return float_step(it, b, beg, end, unit, false);
    }
    let Value::Int(start) = recv else { return Ok(()) };
    let Value::Int(st) = step else {
        return Err(coerce_err(step, "Integer"));
    };
    if *st == 0 {
        return fail(ARGUMENT_ERROR, "step can't be 0");
    }
    let lim = match limit {
        Value::Nil => None,
        Value::Int(l) => Some(*l),
        other => return Err(comparison_failed(recv, other)),
    };
    let mut k = *start;
    loop {
        if let Some(l) = lim {
            if (*st > 0 && k > l) || (*st < 0 && k < l) {
                break;
            }
        }
        it.yield1(b, Value::Int(k))?;
        match k.checked_add(*st) {
            Some(n) => k = n,
            None => break,
        }
    }
    Ok(())
}

pub fn call_float(
    it: &mut Interp,
    f: f64,
    recv: &Value,
    name: &str,
    args: Vec<Value>,
    blk: Option<&BlockArg>,
) -> R<Value> {
    match name {
        "+" | "-" | "*" | "/" => {
            argc(&args, 1, 1)?;
            let g = to_f(&args[0], "Float")?;
            Ok(Value::Float(match name {
                "+" => f + g,
                "-" => f - g,
                "*" => f * g,
                _ => f / g,
            }))
        }
        "%" => {
            argc(&args, 1, 1)?;
            let g = to_f(&args[0], "Float")?;
            Ok(Value::Float(float_mod(f, g)?))
        }
        "**" => {
            argc(&args, 1, 1)?;
            let g = to_f(&args[0], "Float")?;
            float_pow(f, g)
        }
        "-@" => {
            argc(&args, 0, 0)?;
            Ok(Value::Float(-f))
        }
        "<=>" | "<" | "<=" | ">" | ">=" => cmp_op(recv, name, &args),
        "abs" => {
            argc(&args, 0, 0)?;
            Ok(Value::Float(f.abs()))
        }
        "to_f" => {
            argc(&args, 0, 0)?;
            Ok(recv.clone())
        }
        "to_i" => {
            argc(&args, 0, 0)?;
            Ok(Value::Int(float_to_int(f)?))
        }
        "truncate" => {
            let nd = ndigits_arg(&args)?;
            if nd > 0 {
                return float_floor_ceil(f, nd, f < 0.0);
            }
            let v = float_to_int(f)?;
            Ok(Value::Int(int_round_digits(v, nd, 't')?))
        }
        "nan?" => {
            argc(&args, 0, 0)?;
            Ok(Value::Bool(f.is_nan()))
        }
        "finite?" => {
            argc(&args, 0, 0)?;
            Ok(Value::Bool(f.is_finite()))
        }
        "infinite?" => {
            argc(&args, 0, 0)?;
            Ok(if f.is_infinite() {
                Value::Int(if f > 0.0 { 1 } else { -1 })
            } else {
                Value::Nil
            })
        }
        "zero?" | "positive?" | "negative?" => {
            argc(&args, 0, 0)?;
            Ok(Value::Bool(match name {
                "zero?" => f == 0.0,
                "positive?" => f > 0.0,
                _ => f < 0.0,
            }))
        }
        "between?" => between(recv, &args),
        "clamp" => clamp(recv, &args),
        "divmod" => {
            argc(&args, 1, 1)?;
            let g = to_f(&args[0], "Float")?;
            let (d, m) = flo_divmod(f, g)?;
            Ok(Value::array(vec![Value::Int(float_to_int(d)?), Value::Float(m)]))
        }
        "fdiv" => {
            argc(&args, 1, 1)?;
            Ok(Value::Float(f / to_f(&args[0], "Float")?))
        }
        "floor" | "ceil" => {
            let nd = ndigits_arg(&args)?;
            float_floor_ceil(f, nd, name == "ceil")
        }
        "round" => {
            let nd = ndigits_arg(&args)?;
            if nd > 0 {
                if !f.is_finite() {
                    return Ok(recv.clone());
                }
                return Ok(Value::Float(float_round_pos(f, nd)));
            }
            if f == 0.0 {
                return Ok(Value::Int(0));
            }
            if nd == 0 {
                return Ok(Value::Int(float_to_int(round_half_up(f, 1.0))?));
            }
            let v = float_to_int(f)?;
            Ok(Value::Int(int_round_digits(v, nd, 'r')?))
        }
        "to_s" => {
            argc(&args, 0, 0)?;
            Ok(Value::string(float_to_s(f)))
        }
        _ => object::call(it, recv, name, args, blk),
    }
}
