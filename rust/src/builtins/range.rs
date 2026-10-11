//! Range methods (§8.10).

use super::numeric::{float_step, int_overflow};
use super::{argc, block_or_enum, check_array_len, enumerable, object, to_int};
use crate::error::{fail, R, ARGUMENT_ERROR, RANGE_ERROR, TYPE_ERROR};
use crate::interp::{BlockArg, Interp};
use crate::value::{RangeVal, Value};
use std::cmp::Ordering;

/// `rb_range_beg_len`: start and length of a range applied to a sequence of `len` items,
/// or `None` when the start is out of bounds.
pub fn beg_len(r: &RangeVal, len: i64) -> R<Option<(i64, i64)>> {
    let mut beg = match &r.start {
        Value::Nil => 0,
        v => to_int(v)?,
    };
    let (mut end, excl) = match &r.end {
        Value::Nil => (len, true),
        v => (to_int(v)?, r.excl),
    };
    if beg < 0 {
        beg += len;
        if beg < 0 {
            return Ok(None);
        }
    }
    if beg > len {
        return Ok(None);
    }
    if end < 0 {
        end += len;
    }
    if !excl {
        end = end.saturating_add(1);
    }
    let mut l = end - beg;
    if l < 0 {
        l = 0;
    }
    if beg + l > len {
        l = len - beg;
    }
    Ok(Some((beg, l)))
}

/// `Range#cover?` / `===` / `include?`.
pub fn cover(r: &RangeVal, v: &Value) -> bool {
    if !r.start.is_nil() {
        match r.start.compare(v) {
            Some(Ordering::Greater) | None => return false,
            _ => {}
        }
    }
    if !r.end.is_nil() {
        match v.compare(&r.end) {
            Some(Ordering::Less) => {}
            Some(Ordering::Equal) => return !r.excl,
            _ => return false,
        }
    }
    true
}

/// Last integer of an iteration ending at `end` (`None` for an endless range).
pub fn int_end(end: &Value, excl: bool) -> R<Option<i64>> {
    match end {
        Value::Nil => Ok(None),
        Value::Int(e) => Ok(Some(if excl { e.saturating_sub(1) } else { *e })),
        Value::Float(f) => {
            if f.is_nan() {
                return Ok(Some(i64::MIN));
            }
            if *f == f64::INFINITY {
                return Ok(None);
            }
            let x = if excl && f.fract() == 0.0 { f - 1.0 } else { f.floor() };
            if x < -9.2e18 {
                return Ok(Some(i64::MIN));
            }
            Ok(Some(if x > 9.2e18 { i64::MAX } else { x as i64 }))
        }
        other => fail(TYPE_ERROR, format!("can't iterate from {}", other.class_name())),
    }
}

fn int_start(r: &RangeVal) -> R<i64> {
    match &r.start {
        Value::Int(i) => Ok(*i),
        other => fail(TYPE_ERROR, format!("can't iterate from {}", other.class_name())),
    }
}

/// Number of elements of an integer range.
fn int_size(lo: i64, hi: Option<i64>) -> Option<i64> {
    let hi = hi?;
    if hi < lo {
        Some(0)
    } else {
        Some(((hi as i128) - (lo as i128) + 1).min(i64::MAX as i128) as i64)
    }
}

/// All elements of a finite integer range.
pub fn to_vec(v: &Value) -> R<Vec<Value>> {
    let Value::Range(r) = v else { return Ok(vec![]) };
    if r.end.is_nil() {
        return fail(RANGE_ERROR, "cannot convert endless range to an array");
    }
    let lo = int_start(r)?;
    let hi = int_end(&r.end, r.excl)?;
    let n = int_size(lo, hi).unwrap_or(0);
    check_array_len(n as usize)?;
    let mut out = Vec::with_capacity(n as usize);
    let mut i = lo;
    for _ in 0..n {
        out.push(Value::Int(i));
        i = i.wrapping_add(1);
    }
    Ok(out)
}

fn each_int(it: &mut Interp, r: &RangeVal, b: &BlockArg) -> R<()> {
    let lo = int_start(r)?;
    let hi = int_end(&r.end, r.excl)?;
    let mut i = lo;
    while hi.is_none_or(|h| i <= h) {
        it.yield1(b, Value::Int(i))?;
        match i.checked_add(1) {
            Some(n) => i = n,
            None => break,
        }
    }
    Ok(())
}

/// First `n` elements (works for endless ranges).
fn first_n(r: &RangeVal, n: i64) -> R<Vec<Value>> {
    if n < 0 {
        return fail(ARGUMENT_ERROR, "negative array size (or size too big)");
    }
    let lo = int_start(r)?;
    let hi = int_end(&r.end, r.excl)?;
    let mut out = Vec::new();
    let mut i = lo;
    while (out.len() as i64) < n && hi.is_none_or(|h| i <= h) {
        out.push(Value::Int(i));
        check_array_len(out.len())?;
        match i.checked_add(1) {
            Some(x) => i = x,
            None => break,
        }
    }
    Ok(out)
}

pub fn call(it: &mut Interp, recv: &Value, name: &str, args: Vec<Value>, blk: Option<&BlockArg>) -> R<Value> {
    let Value::Range(r) = recv else { return object::call(it, recv, name, args, blk) };
    match name {
        "begin" => {
            argc(&args, 0, 0)?;
            Ok(r.start.clone())
        }
        "end" => {
            argc(&args, 0, 0)?;
            Ok(r.end.clone())
        }
        "first" => {
            argc(&args, 0, 1)?;
            match args.first() {
                None => {
                    if r.start.is_nil() {
                        return fail(RANGE_ERROR, "cannot get the first element of beginless range");
                    }
                    Ok(r.start.clone())
                }
                Some(n) => Ok(Value::array(first_n(r, to_int(n)?)?)),
            }
        }
        "last" => {
            argc(&args, 0, 1)?;
            match args.first() {
                None => {
                    if r.end.is_nil() {
                        return fail(RANGE_ERROR, "cannot get the last element of endless range");
                    }
                    Ok(r.end.clone())
                }
                Some(n) => {
                    let n = to_int(n)?;
                    if n < 0 {
                        return fail(ARGUMENT_ERROR, "negative array size");
                    }
                    let items = to_vec(recv)?;
                    let k = (n as usize).min(items.len());
                    Ok(Value::array(items[items.len() - k..].to_vec()))
                }
            }
        }
        "min" | "max" if args.is_empty() && blk.is_none() => {
            let is_min = name == "min";
            if is_min && r.start.is_nil() {
                return fail(RANGE_ERROR, "cannot get the minimum of beginless range");
            }
            if !is_min && r.end.is_nil() {
                return fail(RANGE_ERROR, "cannot get the maximum of endless range");
            }
            let c = if r.start.is_nil() || r.end.is_nil() { Some(Ordering::Less) } else { r.start.compare(&r.end) };
            let Some(c) = c else {
                return Err(super::numeric::comparison_failed(&r.start, &r.end));
            };
            if c == Ordering::Greater || (is_min && c == Ordering::Equal && r.excl) {
                return Ok(Value::Nil);
            }
            if is_min {
                return Ok(r.start.clone());
            }
            if r.excl {
                let Value::Int(e) = &r.end else {
                    return fail(TYPE_ERROR, "cannot exclude non Integer end value");
                };
                if c == Ordering::Equal {
                    return Ok(Value::Nil);
                }
                if !matches!(r.start, Value::Int(_)) {
                    return fail(TYPE_ERROR, "cannot exclude end value with non Integer begin value");
                }
                return Ok(Value::Int(e - 1));
            }
            Ok(r.end.clone())
        }
        "size" | "count" if args.is_empty() && blk.is_none() => {
            match &r.start {
                Value::Int(lo) => {
                    let hi = int_end(&r.end, r.excl)?;
                    Ok(match int_size(*lo, hi) {
                        Some(n) => Value::Int(n),
                        None => Value::Float(f64::INFINITY),
                    })
                }
                Value::Nil if name == "count" => Ok(Value::Float(f64::INFINITY)),
                other => fail(TYPE_ERROR, format!("can't iterate from {}", other.class_name())),
            }
        }
        "sum" if blk.is_none() && args.is_empty() => {
            if let (Value::Int(lo), Value::Int(_)) = (&r.start, &r.end) {
                let hi = int_end(&r.end, r.excl)?.unwrap_or(*lo - 1);
                if hi < *lo {
                    return Ok(Value::Int(0));
                }
                let n = (hi as i128) - (*lo as i128) + 1;
                let total = (*lo as i128 + hi as i128) * n / 2;
                return i64::try_from(total).map(Value::Int).or_else(|_| int_overflow());
            }
            let items = to_vec(recv)?;
            enumerable::call(it, recv, items, name, args, blk)
        }
        "to_a" | "entries" => {
            argc(&args, 0, 0)?;
            Ok(Value::array(to_vec(recv)?))
        }
        "each" => {
            argc(&args, 0, 0)?;
            let b = block_or_enum!(blk, recv, name, args);
            each_int(it, r, b)?;
            Ok(recv.clone())
        }
        "reverse_each" => {
            argc(&args, 0, 0)?;
            let b = block_or_enum!(blk, recv, name, args);
            let lo = int_start(r)?;
            let Some(hi) = int_end(&r.end, r.excl)? else {
                return fail(TYPE_ERROR, "can't iterate from NilClass");
            };
            let mut i = hi;
            while i >= lo {
                it.yield1(b, Value::Int(i))?;
                match i.checked_sub(1) {
                    Some(n) => i = n,
                    None => break,
                }
            }
            Ok(recv.clone())
        }
        "step" => {
            argc(&args, 1, 1)?;
            let b = block_or_enum!(blk, recv, name, args);
            match (&r.start, &args[0]) {
                (Value::Int(lo), Value::Int(st)) if !matches!(r.end, Value::Float(_)) => {
                    if *st == 0 {
                        return fail(ARGUMENT_ERROR, "step can't be 0");
                    }
                    let end = match &r.end {
                        Value::Nil => None,
                        Value::Int(e) => Some(*e),
                        _ => None,
                    };
                    let mut i = *lo;
                    loop {
                        if let Some(e) = end {
                            let past = if *st > 0 { i > e || (r.excl && i == e) } else { i < e || (r.excl && i == e) };
                            if past {
                                break;
                            }
                        }
                        it.yield1(b, Value::Int(i))?;
                        match i.checked_add(*st) {
                            Some(n) => i = n,
                            None => break,
                        }
                    }
                }
                (Value::Int(_) | Value::Float(_), Value::Int(_) | Value::Float(_)) => {
                    let f = |v: &Value| match v {
                        Value::Int(i) => *i as f64,
                        Value::Float(f) => *f,
                        _ => f64::INFINITY,
                    };
                    let unit = f(&args[0]);
                    if unit == 0.0 {
                        return fail(ARGUMENT_ERROR, "step can't be 0");
                    }
                    let end = if r.end.is_nil() { if unit < 0.0 { f64::NEG_INFINITY } else { f64::INFINITY } } else { f(&r.end) };
                    float_step(it, b, f(&r.start), end, unit, r.excl)?;
                }
                (s, _) => return fail(TYPE_ERROR, format!("can't iterate from {}", s.class_name())),
            }
            Ok(recv.clone())
        }
        "include?" | "member?" if matches!((&r.start, &r.end), (Value::Str(_), Value::Str(_))) => {
            argc(&args, 1, 1)?;
            // Ruby iterates string ranges with String#succ, except for one-character ends.
            let (Value::Str(b), Value::Str(e)) = (&r.start, &r.end) else { return Ok(Value::Bool(false)) };
            let Value::Str(v) = &args[0] else { return Ok(Value::Bool(false)) };
            if b.len() == 1 && e.len() == 1 && b.is_ascii() && e.is_ascii() {
                if v.len() != 1 {
                    return Ok(Value::Bool(false));
                }
                return Ok(Value::Bool(cover(r, &args[0])));
            }
            fail(TYPE_ERROR, "can't iterate from String")
        }
        "include?" | "member?" | "cover?" => {
            argc(&args, 1, 1)?;
            Ok(Value::Bool(cover(r, &args[0])))
        }
        "exclude_end?" => {
            argc(&args, 0, 0)?;
            Ok(Value::Bool(r.excl))
        }
        _ => {
            if super::ENUMERABLE_METHODS.contains(&name) {
                if let ("first", Value::Nil) = (name, &r.end) {
                    return object::call(it, recv, name, args, blk);
                }
                let items = to_vec(recv)?;
                return enumerable::call(it, recv, items, name, args, blk);
            }
            object::call(it, recv, name, args, blk)
        }
    }
}
