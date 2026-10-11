// Copyright 2026 OpenC3, Inc.
// All Rights Reserved.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
// See LICENSE.md for more details.

//! Built-in functions (§9): `format`, `Integer()`, `render`, ... and the class/module
//! methods of `Math`, `JSON`, `File`, `Dir`, `Array.new`, `Hash.new`.

use super::conv::{str_to_float_strict, str_to_int_strict};
use super::{argc, check_array_len, file, object, to_int, to_str};
use crate::error::{
    fail, ARGUMENT_ERROR, MATH_DOMAIN_ERROR, NAME_ERROR, NO_METHOD_ERROR, R, RUNTIME_ERROR, TYPE_ERROR,
};
use crate::format::{float_to_int, sprintf};
use crate::interp::{BlockArg, Interp};
use crate::value::{ClassId, RHash, Value};
use std::cmp::Ordering;
use std::sync::Arc;

/// Calls a bare function (an identifier that is not a local or host variable).
pub fn call_function(it: &mut Interp, name: &str, args: Vec<Value>, blk: Option<&BlockArg>, bare: bool) -> R<Value> {
    match name {
        "format" | "sprintf" => {
            argc(&args, 1, usize::MAX)?;
            let fmt = to_str(&args[0])?;
            Ok(Value::string(sprintf(&fmt, &args[1..])?))
        }
        "Integer" => {
            argc(&args, 1, 2)?;
            let base = match args.get(1) {
                Some(Value::Nil) | None => None,
                Some(b) => Some(to_int(b)?),
            };
            match &args[0] {
                Value::Str(s) => Ok(Value::Int(str_to_int_strict(s, base)?)),
                _ if base.is_some() => fail(ARGUMENT_ERROR, "base specified for non string value"),
                Value::Int(_) => Ok(args[0].clone()),
                Value::Float(f) => Ok(Value::Int(float_to_int(*f)?)),
                other => fail(TYPE_ERROR, format!("can't convert {} into Integer", other.type_desc())),
            }
        }
        "Float" => {
            argc(&args, 1, 1)?;
            match &args[0] {
                Value::Int(i) => Ok(Value::Float(*i as f64)),
                Value::Float(_) => Ok(args[0].clone()),
                Value::Str(s) => Ok(Value::Float(str_to_float_strict(s)?)),
                other => fail(TYPE_ERROR, format!("can't convert {} into Float", other.type_desc())),
            }
        }
        "String" => {
            argc(&args, 1, 1)?;
            Ok(match &args[0] {
                Value::Str(_) => args[0].clone(),
                other => Value::string(other.to_s()?),
            })
        }
        "Array" => {
            argc(&args, 1, 1)?;
            Ok(match &args[0] {
                Value::Nil => Value::array(vec![]),
                Value::Array(_) => args[0].clone(),
                Value::Hash(_) | Value::Range(_) | Value::Enumerator(_) => {
                    Value::array(super::iter_values(it, &args[0])?)
                }
                other => Value::array(vec![other.clone()]),
            })
        }
        "raise" | "fail" => {
            argc(&args, 0, 1)?;
            match args.first() {
                None => fail(RUNTIME_ERROR, "unhandled exception"),
                Some(Value::Str(s)) => fail(RUNTIME_ERROR, s.to_string()),
                Some(_) => fail(TYPE_ERROR, "exception class/object expected"),
            }
        }
        "loop" => {
            argc(&args, 0, 0)?;
            let Some(b) = blk else {
                return fail(ARGUMENT_ERROR, "no block given (loop)");
            };
            loop {
                it.tick()?;
                it.call_block(b, vec![])?;
            }
        }
        "render" => {
            argc(&args, 1, 2)?;
            let path = to_str(&args[0])?;
            let mut locals: Vec<(Arc<str>, Value)> = Vec::new();
            match args.get(1) {
                None | Some(Value::Nil) => {}
                Some(Value::Hash(h)) => {
                    for (k, v) in h.borrow().iter() {
                        match k {
                            Value::Sym(s) | Value::Str(s) => locals.push((s.clone(), v.clone())),
                            other => {
                                return fail(
                                    TYPE_ERROR,
                                    format!(
                                        "render locals keys must be Symbols or Strings, got {}",
                                        other.class_name()
                                    ),
                                )
                            }
                        }
                    }
                }
                Some(other) => {
                    return fail(
                        TYPE_ERROR,
                        format!("no implicit conversion of {} into Hash", other.type_desc()),
                    );
                }
            }
            let source = file::read(it, &path)?;
            let out = it.render_partial(&path, &source, locals)?;
            Ok(Value::string(out))
        }
        _ => {
            if bare {
                fail(
                    NAME_ERROR,
                    format!("undefined local variable or method '{}' for main", name),
                )
            } else {
                fail(NO_METHOD_ERROR, format!("undefined method '{}' for main", name))
            }
        }
    }
}

/// `Base::NAME` constant lookup.
pub fn scoped_const(base: &Value, name: &str) -> R<Value> {
    match (base, name) {
        (Value::Class(ClassId::Float), "INFINITY") => Ok(Value::Float(f64::INFINITY)),
        (Value::Class(ClassId::Float), "NAN") => Ok(Value::Float(f64::NAN)),
        (Value::Class(ClassId::Float), "EPSILON") => Ok(Value::Float(f64::EPSILON)),
        (Value::Class(ClassId::Float), "MAX") => Ok(Value::Float(f64::MAX)),
        (Value::Class(ClassId::Float), "MIN") => Ok(Value::Float(f64::MIN_POSITIVE)),
        (Value::Class(ClassId::Math), "PI") => Ok(Value::Float(std::f64::consts::PI)),
        (Value::Class(ClassId::Math), "E") => Ok(Value::Float(std::f64::consts::E)),
        (Value::Class(c), _) => fail(NAME_ERROR, format!("uninitialized constant {}::{}", c.name(), name)),
        (other, _) => fail(TYPE_ERROR, format!("{} is not a class/module", other.inspect()?)),
    }
}

fn math_arg(v: &Value) -> R<f64> {
    match v {
        Value::Int(i) => Ok(*i as f64),
        Value::Float(f) => Ok(*f),
        other => fail(TYPE_ERROR, format!("can't convert {} into Float", other.type_desc())),
    }
}

fn domain<T>(name: &str) -> R<T> {
    fail(
        MATH_DOMAIN_ERROR,
        format!("Numerical argument is out of domain - \"{}\"", name),
    )
}

/// Correctly rounded cube root [ERBLite]. Platform `cbrt()` results are off by one
/// ulp for some inputs, differently per platform. Start from `f64::cbrt` and step to
/// the double whose rounding interval contains the exact root, comparing cubes of the
/// interval midpoints exactly. A midpoint's cube is never exactly a double, so there
/// are no ties.
fn cbrt(x: f64) -> f64 {
    if x == 0.0 || !x.is_finite() {
        return x;
    }
    if x < 0.0 {
        return -cbrt(-x);
    }
    let mut y = x.cbrt();
    loop {
        // y > 0 and far from the f64 range limits, so stepping the bits moves one ulp.
        let up = f64::from_bits(y.to_bits() + 1);
        if cmp_midpoint_cube(y, up, x) == Ordering::Less {
            y = up;
            continue;
        }
        let down = f64::from_bits(y.to_bits() - 1);
        if cmp_midpoint_cube(down, y, x) == Ordering::Greater {
            y = down;
            continue;
        }
        return y;
    }
}

/// Positive finite `v` as `m * 2^e`.
fn decompose(v: f64) -> (u64, i64) {
    let bits = v.to_bits();
    let exp = ((bits >> 52) & 0x7ff) as i64;
    let frac = bits & ((1 << 52) - 1);
    if exp == 0 {
        (frac, -1074)
    } else {
        (frac | (1 << 52), exp - 1075)
    }
}

/// Exactly compares `((a + b) / 2)^3` with `x` for adjacent positive doubles `a < b`.
fn cmp_midpoint_cube(a: f64, b: f64, x: f64) -> Ordering {
    let (ma, ea) = decompose(a);
    let (mb, eb) = decompose(b);
    let e = ea.min(eb);
    // Mantissas are < 2^53 and exponents differ by at most 1, so the sum is < 2^55.
    let sum = (ma << (ea - e)) + (mb << (eb - e));
    let mut cube = [sum, 0, 0, 0, 0];
    big_mul(&mut cube, sum);
    big_mul(&mut cube, sum);
    let (mx, ex) = decompose(x);
    cmp_scaled(&cube, 3 * (e - 1), &[mx, 0, 0, 0, 0], ex)
}

type Big = [u64; 5];

fn big_mul(n: &mut Big, m: u64) {
    let mut carry = 0u128;
    for limb in n.iter_mut() {
        let t = (*limb as u128) * (m as u128) + carry;
        *limb = t as u64;
        carry = t >> 64;
    }
}

fn big_bits(n: &Big) -> i64 {
    match n.iter().rposition(|&l| l != 0) {
        Some(i) => (i as i64) * 64 + 64 - n[i].leading_zeros() as i64,
        None => 0,
    }
}

fn big_shl(n: &Big, shift: i64) -> Big {
    let (words, bits) = ((shift / 64) as usize, (shift % 64) as u32);
    let mut out = [0u64; 5];
    for i in (words..5).rev() {
        let src = i - words;
        out[i] = n[src] << bits;
        if bits > 0 && src > 0 {
            out[i] |= n[src - 1] >> (64 - bits);
        }
    }
    out
}

/// Compares `a * 2^ea` with `b * 2^eb` (both nonzero).
fn cmp_scaled(a: &Big, ea: i64, b: &Big, eb: i64) -> Ordering {
    let (la, lb) = (big_bits(a) + ea, big_bits(b) + eb);
    if la != lb {
        return la.cmp(&lb);
    }
    // Same magnitude: aligning the exponents keeps both within the bit length of
    // the longer operand, which fits in a Big.
    let (a, b) = if ea > eb {
        (big_shl(a, ea - eb), *b)
    } else {
        (*a, big_shl(b, eb - ea))
    };
    a.iter().rev().cmp(b.iter().rev())
}

fn math(name: &str, args: &[Value]) -> R<Value> {
    let two = matches!(name, "atan2" | "hypot");
    if name == "log" {
        argc(args, 1, 2)?;
    } else if two {
        argc(args, 2, 2)?;
    } else {
        argc(args, 1, 1)?;
    }
    let x = math_arg(&args[0])?;
    let r = match name {
        "sqrt" => {
            if x < 0.0 {
                return domain(name);
            }
            x.sqrt()
        }
        "cbrt" => cbrt(x),
        "sin" => x.sin(),
        "cos" => x.cos(),
        "tan" => x.tan(),
        "asin" | "acos" => {
            if !(-1.0..=1.0).contains(&x) {
                return domain(name);
            }
            if name == "asin" {
                x.asin()
            } else {
                x.acos()
            }
        }
        "atan" => x.atan(),
        "exp" => x.exp(),
        "atan2" => x.atan2(math_arg(&args[1])?),
        "hypot" => x.hypot(math_arg(&args[1])?),
        "log2" | "log10" => {
            if x < 0.0 {
                return domain(name);
            }
            if name == "log2" {
                x.log2()
            } else {
                x.log10()
            }
        }
        _ => {
            // log(x, base = e)
            if x < 0.0 {
                return domain("log");
            }
            match args.get(1) {
                None => x.ln(),
                Some(b) => {
                    let b = math_arg(b)?;
                    if b < 0.0 {
                        return domain("log");
                    }
                    if x == 0.0 {
                        if b != 0.0 {
                            f64::NEG_INFINITY
                        } else {
                            f64::NAN
                        }
                    } else if b == 0.0 {
                        -0.0
                    } else {
                        x.log2() / b.log2()
                    }
                }
            }
        }
    };
    Ok(Value::Float(r))
}

fn json_opts(args: &[Value]) -> R<bool> {
    match args.get(1) {
        None | Some(Value::Nil) => Ok(false),
        Some(Value::Hash(h)) => Ok(h
            .borrow()
            .get(&Value::sym("symbolize_names"))
            .is_some_and(|v| v.truthy())),
        Some(other) => fail(
            TYPE_ERROR,
            format!("no implicit conversion of {} into Hash", other.type_desc()),
        ),
    }
}

/// Methods called on class and module values.
pub fn call_class(
    it: &mut Interp,
    c: ClassId,
    recv: &Value,
    name: &str,
    args: Vec<Value>,
    blk: Option<&BlockArg>,
) -> R<Value> {
    match (c, name) {
        (_, "name") => {
            argc(&args, 0, 0)?;
            Ok(Value::str(c.name()))
        }
        (ClassId::Array, "new") => {
            argc(&args, 0, 2)?;
            let n = match args.first() {
                Some(v) => to_int(v)?,
                None => 0,
            };
            if n < 0 {
                return fail(ARGUMENT_ERROR, "negative array size");
            }
            check_array_len(n as usize)?;
            let mut out = Vec::with_capacity(n as usize);
            for i in 0..n {
                out.push(match blk {
                    Some(b) => it.yield_keep(b, Value::Int(i))?,
                    None => args.get(1).cloned().unwrap_or(Value::Nil),
                });
            }
            Ok(Value::array(out))
        }
        (ClassId::Hash, "new") => {
            argc(&args, 0, 1)?;
            if blk.is_some() {
                return fail(ARGUMENT_ERROR, "Hash.new with a block is not supported");
            }
            let mut h = RHash::new();
            h.default = args.first().cloned().unwrap_or(Value::Nil);
            Ok(Value::hash(h))
        }
        (ClassId::Math, _) if super::MATH_FUNCTIONS.contains(&name) => math(name, &args),
        (ClassId::Json, "parse") => {
            argc(&args, 1, 2)?;
            let text = to_str(&args[0])?;
            crate::json::parse(&text, json_opts(&args)?)
        }
        (ClassId::Json, "generate" | "pretty_generate") => {
            argc(&args, 1, 2)?;
            json_opts(&args)?;
            let s = if name == "generate" {
                crate::json::generate(&args[0])?
            } else {
                crate::json::pretty_generate(&args[0])?
            };
            Ok(Value::string(s))
        }
        (ClassId::File, _) if super::FILE_FUNCTIONS.contains(&name) => file::call_file(it, name, &args),
        (ClassId::Dir, "glob") => {
            argc(&args, 1, 1)?;
            let pat = to_str(&args[0])?;
            Ok(Value::array(
                file::glob(it, &pat)?.into_iter().map(Value::string).collect(),
            ))
        }
        _ => object::call(it, recv, name, args, blk),
    }
}
