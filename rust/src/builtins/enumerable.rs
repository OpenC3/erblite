// Copyright 2026 OpenC3, Inc.
// All Rights Reserved.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
// See LICENSE.md for more details.

//! Enumerable methods (§8.8), implemented over a snapshot of the yielded values.

use super::numeric::{compare_or_fail, comparison_failed, int_overflow};
use super::object::case_eq;
use super::{argc, block_or_enum, check_array_len, iter_values, no_method, object, to_int};
use crate::error::{fail, ARGUMENT_ERROR, R, TYPE_ERROR};
use crate::interp::{BlockArg, Interp};
use crate::value::{RHash, Value};
use std::cmp::Ordering;
use std::collections::HashSet;
use std::sync::Arc;

type Cmp<'a> = dyn FnMut(&mut Interp, &Value, &Value) -> R<Ordering> + 'a;

/// Stable merge sort with a fallible comparator.
pub fn merge_sort(it: &mut Interp, v: Vec<Value>, cmp: &mut Cmp) -> R<Vec<Value>> {
    let n = v.len();
    let mut src = v;
    let mut width = 1;
    while width < n {
        let mut dst = Vec::with_capacity(n);
        let mut i = 0;
        while i < n {
            let mid = (i + width).min(n);
            let hi = (i + 2 * width).min(n);
            let (mut a, mut b) = (i, mid);
            while a < mid && b < hi {
                if cmp(it, &src[b], &src[a])? == Ordering::Less {
                    dst.push(src[b].clone());
                    b += 1;
                } else {
                    dst.push(src[a].clone());
                    a += 1;
                }
            }
            dst.extend_from_slice(&src[a..mid]);
            dst.extend_from_slice(&src[b..hi]);
            i = hi;
        }
        src = dst;
        width *= 2;
    }
    Ok(src)
}

/// Converts a `<=>`-style block result into an ordering.
pub fn cmp_result(r: &Value, a: &Value, b: &Value) -> R<Ordering> {
    match r {
        Value::Int(i) => Ok(i.cmp(&0)),
        Value::Float(f) if !f.is_nan() => Ok(f.partial_cmp(&0.0).unwrap_or(Ordering::Equal)),
        _ => Err(comparison_failed(a, b)),
    }
}

/// Sorts with the block as comparator, or with `<=>`.
pub fn sort_values(it: &mut Interp, items: Vec<Value>, blk: Option<&BlockArg>) -> R<Vec<Value>> {
    match blk {
        Some(b) => merge_sort(it, items, &mut |it: &mut Interp, x: &Value, y: &Value| {
            let r = it.call_block(b, vec![x.clone(), y.clone()])?;
            cmp_result(&r, x, y)
        }),
        None => merge_sort(it, items, &mut |_: &mut Interp, x: &Value, y: &Value| {
            compare_or_fail(x, y)
        }),
    }
}

/// Sorts by keys computed with the block.
pub fn sort_by(it: &mut Interp, items: Vec<Value>, b: &BlockArg) -> R<Vec<Value>> {
    let mut pairs = Vec::with_capacity(items.len());
    for x in items {
        let k = it.yield1(b, x.clone())?;
        pairs.push(Value::array(vec![k, x]));
    }
    let sorted = merge_sort(it, pairs, &mut |_: &mut Interp, p: &Value, q: &Value| {
        let (pk, qk) = (first_of(p), first_of(q));
        compare_or_fail(&pk, &qk)
    })?;
    Ok(sorted.iter().map(second_of).collect())
}

fn first_of(v: &Value) -> Value {
    match v {
        Value::Array(a) => a.borrow().first().cloned().unwrap_or(Value::Nil),
        _ => Value::Nil,
    }
}

fn second_of(v: &Value) -> Value {
    match v {
        Value::Array(a) => a.borrow().get(1).cloned().unwrap_or(Value::Nil),
        _ => Value::Nil,
    }
}

/// Ruby's `sum` (exact integers, then Kahan-Babuska once a Float appears, then `+`).
/// `float_init_kahan` is true for Enumerable#sum (a Float init also uses Kahan).
pub fn sum_values(
    it: &mut Interp,
    items: Vec<Value>,
    init: Value,
    blk: Option<&BlockArg>,
    float_init_kahan: bool,
) -> R<Value> {
    let mut iter = items.into_iter();
    let mut v = init;
    let next = |it: &mut Interp, iter: &mut std::vec::IntoIter<Value>| -> R<Option<Value>> {
        match iter.next() {
            None => Ok(None),
            Some(x) => match blk {
                Some(b) => Ok(Some(it.yield1(b, x)?)),
                None => Ok(Some(x)),
            },
        }
    };
    let mut pending: Option<Value> = None;
    let mut float_phase: Option<f64> = None;
    match &v {
        Value::Int(_) => loop {
            let Some(e) = next(it, &mut iter)? else { return Ok(v) };
            match (&v, &e) {
                (Value::Int(a), Value::Int(b)) => match a.checked_add(*b) {
                    Some(s) => v = Value::Int(s),
                    None => return int_overflow(),
                },
                (Value::Int(a), Value::Float(_)) => {
                    float_phase = Some(*a as f64);
                    pending = Some(e);
                    break;
                }
                _ => {
                    pending = Some(e);
                    break;
                }
            }
        },
        Value::Float(f) if float_init_kahan => {
            float_phase = Some(*f);
        }
        _ => {}
    }
    if let Some(mut f) = float_phase {
        let mut c = 0.0f64;
        loop {
            let e = match pending.take() {
                Some(e) => e,
                None => match next(it, &mut iter)? {
                    Some(e) => e,
                    None => return Ok(Value::Float(f + c)),
                },
            };
            let x = match &e {
                Value::Float(x) => *x,
                Value::Int(i) => *i as f64,
                _ => {
                    v = Value::Float(f);
                    pending = Some(e);
                    break;
                }
            };
            if f.is_nan() {
                continue;
            }
            if x.is_nan() {
                f = x;
                continue;
            }
            if x.is_infinite() {
                if f.is_infinite() && x.is_sign_negative() != f.is_sign_negative() {
                    f = f64::NAN;
                } else {
                    f = x;
                }
                continue;
            }
            if f.is_infinite() {
                continue;
            }
            let t = f + x;
            if f.abs() >= x.abs() {
                c += (f - t) + x;
            } else {
                c += (x - t) + f;
            }
            f = t;
        }
    }
    loop {
        let e = match pending.take() {
            Some(e) => e,
            None => match next(it, &mut iter)? {
                Some(e) => e,
                None => return Ok(v),
            },
        };
        v = it.call_method(&v, "+", vec![e], None)?;
    }
}

fn hkey_uniq(it: &mut Interp, items: Vec<Value>, blk: Option<&BlockArg>) -> R<Vec<Value>> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for x in items {
        let key = match blk {
            Some(b) => it.yield1(b, x.clone())?,
            None => x.clone(),
        };
        if seen.insert(key.hkey()) {
            out.push(x);
        }
    }
    Ok(out)
}

pub fn uniq(it: &mut Interp, items: Vec<Value>, blk: Option<&BlockArg>) -> R<Vec<Value>> {
    hkey_uniq(it, items, blk)
}

/// Pairs (`[k, v]` arrays) to a Hash.
pub fn pairs_to_hash(items: &[Value]) -> R<Value> {
    let mut h = RHash::new();
    for (i, x) in items.iter().enumerate() {
        let Value::Array(a) = x else {
            return fail(
                TYPE_ERROR,
                format!("wrong element type {} at {} (expected array)", x.class_name(), i),
            );
        };
        let a = a.borrow();
        if a.len() != 2 {
            return fail(
                ARGUMENT_ERROR,
                format!("wrong array length at {} (expected 2, was {})", i, a.len()),
            );
        }
        h.insert(a[0].clone(), a[1].clone());
    }
    Ok(Value::hash(h))
}

fn take_n(args: &[Value], what: &str) -> R<usize> {
    let n = to_int(&args[0])?;
    if n < 0 {
        return fail(ARGUMENT_ERROR, format!("attempt to {} negative size", what));
    }
    Ok(n as usize)
}

fn extreme(it: &mut Interp, items: Vec<Value>, args: &[Value], blk: Option<&BlockArg>, is_min: bool) -> R<Value> {
    argc(args, 0, 1)?;
    if let Some(n) = args.first() {
        let n = to_int(n)?;
        if n < 0 {
            return fail(ARGUMENT_ERROR, format!("negative size ({})", n));
        }
        let mut sorted = sort_values(it, items, blk)?;
        if !is_min {
            sorted.reverse();
        }
        sorted.truncate(n as usize);
        return Ok(Value::array(sorted));
    }
    let mut best: Option<Value> = None;
    for x in items {
        best = Some(match best {
            None => x,
            Some(cur) => {
                let o = match blk {
                    Some(b) => {
                        let r = it.call_block(b, vec![x.clone(), cur.clone()])?;
                        cmp_result(&r, &x, &cur)?
                    }
                    None => compare_or_fail(&x, &cur)?,
                };
                if (is_min && o == Ordering::Less) || (!is_min && o == Ordering::Greater) {
                    x
                } else {
                    cur
                }
            }
        });
    }
    Ok(best.unwrap_or(Value::Nil))
}

fn extreme_by(
    it: &mut Interp,
    recv: &Value,
    items: Vec<Value>,
    name: &str,
    args: &[Value],
    blk: Option<&BlockArg>,
) -> R<Value> {
    argc(args, 0, 1)?;
    let b = block_or_enum!(blk, recv, name, args.to_vec());
    let is_min = name == "min_by";
    if let Some(n) = args.first() {
        let n = to_int(n)?;
        let mut sorted = sort_by(it, items, b)?;
        if !is_min {
            sorted.reverse();
        }
        sorted.truncate(n.max(0) as usize);
        return Ok(Value::array(sorted));
    }
    let mut best: Option<(Value, Value)> = None;
    for x in items {
        let k = it.yield1(b, x.clone())?;
        best = Some(match best {
            None => (k, x),
            Some((bk, bx)) => {
                let o = compare_or_fail(&k, &bk)?;
                if (is_min && o == Ordering::Less) || (!is_min && o == Ordering::Greater) {
                    (k, x)
                } else {
                    (bk, bx)
                }
            }
        });
    }
    Ok(best.map(|(_, x)| x).unwrap_or(Value::Nil))
}

/// Dispatches an Enumerable method over `items` (the values `recv` yields).
pub fn call(
    it: &mut Interp,
    recv: &Value,
    items: Vec<Value>,
    name: &str,
    args: Vec<Value>,
    blk: Option<&BlockArg>,
) -> R<Value> {
    match name {
        "each_with_index" => {
            argc(&args, 0, 0)?;
            let b = block_or_enum!(blk, recv, name, args);
            for (i, x) in items.into_iter().enumerate() {
                it.call_block(b, vec![x, Value::Int(i as i64)])?;
            }
            Ok(recv.clone())
        }
        "each_with_object" => {
            argc(&args, 1, 1)?;
            let b = block_or_enum!(blk, recv, name, args);
            let memo = args[0].clone();
            for x in items {
                it.call_block(b, vec![x, memo.clone()])?;
            }
            Ok(memo)
        }
        "each_slice" | "each_cons" => {
            argc(&args, 1, 1)?;
            let n = to_int(&args[0])?;
            if n <= 0 {
                return fail(
                    ARGUMENT_ERROR,
                    if name == "each_slice" {
                        "invalid slice size"
                    } else {
                        "invalid size"
                    },
                );
            }
            let b = block_or_enum!(blk, recv, name, args);
            let n = n as usize;
            if name == "each_slice" {
                for chunk in items.chunks(n) {
                    it.yield1(b, Value::array(chunk.to_vec()))?;
                }
            } else if items.len() >= n {
                for w in items.windows(n) {
                    it.yield1(b, Value::array(w.to_vec()))?;
                }
            }
            Ok(recv.clone())
        }
        "map" | "collect" | "flat_map" | "collect_concat" | "filter_map" => {
            argc(&args, 0, 0)?;
            let b = block_or_enum!(blk, recv, name, args);
            let mut out = Vec::with_capacity(items.len());
            for x in items {
                let r = it.yield_keep(b, x)?;
                match name {
                    "flat_map" | "collect_concat" => match &r {
                        Value::Array(a) => {
                            let inner = a.borrow().clone();
                            out.extend(inner);
                            check_array_len(out.len())?;
                        }
                        _ => out.push(r),
                    },
                    "filter_map" => {
                        if r.truthy() {
                            out.push(r)
                        }
                    }
                    _ => out.push(r),
                }
            }
            Ok(Value::array(out))
        }
        "select" | "filter" | "find_all" | "reject" => {
            argc(&args, 0, 0)?;
            let b = block_or_enum!(blk, recv, name, args);
            let keep = name != "reject";
            let mut out = Vec::new();
            for x in items {
                if it.yield1(b, x.clone())?.truthy() == keep {
                    out.push(x);
                }
            }
            Ok(Value::array(out))
        }
        "find" | "detect" => {
            argc(&args, 0, 1)?;
            let b = block_or_enum!(blk, recv, name, args);
            for x in items {
                if it.yield1(b, x.clone())?.truthy() {
                    return Ok(x);
                }
            }
            Ok(Value::Nil)
        }
        "find_index" => {
            argc(&args, 0, 1)?;
            if let Some(v) = args.first() {
                return Ok(items
                    .iter()
                    .position(|x| x.equals(v))
                    .map(|i| Value::Int(i as i64))
                    .unwrap_or(Value::Nil));
            }
            let b = block_or_enum!(blk, recv, name, args);
            for (i, x) in items.into_iter().enumerate() {
                if it.yield1(b, x)?.truthy() {
                    return Ok(Value::Int(i as i64));
                }
            }
            Ok(Value::Nil)
        }
        "any?" | "all?" | "none?" | "one?" => {
            argc(&args, 0, 1)?;
            let mut count = 0usize;
            let mut all = true;
            for x in items {
                let t = if let Some(p) = args.first() {
                    case_eq(p, &x)
                } else if let Some(b) = blk {
                    it.yield1(b, x)?.truthy()
                } else {
                    x.truthy()
                };
                if t {
                    count += 1;
                    if name == "any?" {
                        return Ok(Value::Bool(true));
                    }
                    if name == "none?" {
                        return Ok(Value::Bool(false));
                    }
                    if name == "one?" && count > 1 {
                        return Ok(Value::Bool(false));
                    }
                } else {
                    all = false;
                    if name == "all?" {
                        return Ok(Value::Bool(false));
                    }
                }
            }
            Ok(Value::Bool(match name {
                "any?" => false,
                "all?" => all,
                "none?" => true,
                _ => count == 1,
            }))
        }
        "count" => {
            argc(&args, 0, 1)?;
            if let Some(v) = args.first() {
                return Ok(Value::Int(items.iter().filter(|x| x.equals(v)).count() as i64));
            }
            if let Some(b) = blk {
                let mut n = 0;
                for x in items {
                    if it.yield1(b, x)?.truthy() {
                        n += 1;
                    }
                }
                return Ok(Value::Int(n));
            }
            Ok(Value::Int(items.len() as i64))
        }
        "include?" | "member?" => {
            argc(&args, 1, 1)?;
            Ok(Value::Bool(items.iter().any(|x| x.equals(&args[0]))))
        }
        "min" => extreme(it, items, &args, blk, true),
        "max" => extreme(it, items, &args, blk, false),
        "min_by" | "max_by" => extreme_by(it, recv, items, name, &args, blk),
        "minmax" => {
            argc(&args, 0, 0)?;
            let mn = extreme(it, items.clone(), &[], blk, true)?;
            let mx = extreme(it, items, &[], blk, false)?;
            Ok(Value::array(vec![mn, mx]))
        }
        "sum" => {
            argc(&args, 0, 1)?;
            let init = args.first().cloned().unwrap_or(Value::Int(0));
            let array_semantics = matches!(recv, Value::Array(_));
            sum_values(it, items, init, blk, !array_semantics)
        }
        "inject" | "reduce" => {
            argc(&args, 0, 2)?;
            let (mut acc, op): (Option<Value>, Option<Arc<str>>) = match (args.len(), blk) {
                (2, _) => (Some(args[0].clone()), Some(sym_name(&args[1])?)),
                (1, Some(_)) => (Some(args[0].clone()), None),
                (1, None) => (None, Some(sym_name(&args[0])?)),
                (_, Some(_)) => (None, None),
                _ => return fail(ARGUMENT_ERROR, "no block given"),
            };
            for x in items {
                acc = Some(match acc {
                    None => x,
                    Some(a) => match (&op, blk) {
                        (Some(m), _) => it.call_method(&a, m, vec![x], None)?,
                        (None, Some(b)) => it.call_block(b, vec![a, x])?,
                        _ => a,
                    },
                });
            }
            Ok(acc.unwrap_or(Value::Nil))
        }
        "group_by" | "partition" => {
            argc(&args, 0, 0)?;
            let b = block_or_enum!(blk, recv, name, args);
            if name == "partition" {
                let (mut yes, mut no) = (Vec::new(), Vec::new());
                for x in items {
                    if it.yield1(b, x.clone())?.truthy() {
                        yes.push(x);
                    } else {
                        no.push(x);
                    }
                }
                return Ok(Value::array(vec![Value::array(yes), Value::array(no)]));
            }
            let mut groups: Vec<(Value, Vec<Value>)> = Vec::new();
            let mut index = std::collections::HashMap::new();
            for x in items {
                let k = it.yield1(b, x.clone())?;
                let hk = k.hkey();
                match index.get(&hk) {
                    Some(&i) => {
                        let g: &mut (Value, Vec<Value>) = &mut groups[i];
                        g.1.push(x);
                    }
                    None => {
                        index.insert(hk, groups.len());
                        groups.push((k, vec![x]));
                    }
                }
            }
            Ok(Value::hash_from(
                groups.into_iter().map(|(k, v)| (k, Value::array(v))).collect(),
            ))
        }
        "chunk_while" | "slice_when" => {
            argc(&args, 0, 0)?;
            let Some(b) = blk else {
                return fail(ARGUMENT_ERROR, "no block given");
            };
            let mut out: Vec<Vec<Value>> = Vec::new();
            let mut prev: Option<Value> = None;
            for x in items {
                match prev {
                    None => out.push(vec![x.clone()]),
                    Some(p) => {
                        let r = it.call_block(b, vec![p, x.clone()])?.truthy();
                        let split = if name == "chunk_while" { !r } else { r };
                        if split {
                            out.push(vec![x.clone()]);
                        } else if let Some(last) = out.last_mut() {
                            last.push(x.clone());
                        }
                    }
                }
                prev = Some(x);
            }
            Ok(Value::array(out.into_iter().map(Value::array).collect()))
        }
        "tally" => {
            argc(&args, 0, 0)?;
            let mut h = RHash::new();
            for x in items {
                let n = match h.get(&x) {
                    Some(Value::Int(n)) => n + 1,
                    _ => 1,
                };
                h.insert(x, Value::Int(n));
            }
            Ok(Value::hash(h))
        }
        "zip" => {
            let mut others = Vec::new();
            for a in &args {
                match a {
                    Value::Array(_) | Value::Hash(_) | Value::Range(_) | Value::Enumerator(_) => {
                        others.push(iter_values(it, a)?)
                    }
                    other => {
                        return fail(
                            TYPE_ERROR,
                            format!("wrong argument type {} (must respond to :each)", other.class_name()),
                        )
                    }
                }
            }
            let mut rows = Vec::with_capacity(items.len());
            for (i, x) in items.into_iter().enumerate() {
                let mut row = vec![x];
                for o in &others {
                    row.push(o.get(i).cloned().unwrap_or(Value::Nil));
                }
                rows.push(Value::array(row));
            }
            if let Some(b) = blk {
                for r in rows {
                    it.yield1(b, r)?;
                }
                return Ok(Value::Nil);
            }
            Ok(Value::array(rows))
        }
        "take" => {
            argc(&args, 1, 1)?;
            let n = take_n(&args, "take")?;
            Ok(Value::array(items.into_iter().take(n).collect()))
        }
        "drop" => {
            argc(&args, 1, 1)?;
            let n = take_n(&args, "drop")?;
            Ok(Value::array(items.into_iter().skip(n).collect()))
        }
        "take_while" | "drop_while" => {
            argc(&args, 0, 0)?;
            let b = block_or_enum!(blk, recv, name, args);
            let mut idx = items.len();
            for (i, x) in items.iter().enumerate() {
                if !it.yield1(b, x.clone())?.truthy() {
                    idx = i;
                    break;
                }
            }
            Ok(Value::array(if name == "take_while" {
                items[..idx].to_vec()
            } else {
                items[idx..].to_vec()
            }))
        }
        "first" => {
            argc(&args, 0, 1)?;
            if args.is_empty() {
                return Ok(items.into_iter().next().unwrap_or(Value::Nil));
            }
            let n = take_n(&args, "take")?;
            Ok(Value::array(items.into_iter().take(n).collect()))
        }
        "sort" => {
            argc(&args, 0, 0)?;
            Ok(Value::array(sort_values(it, items, blk)?))
        }
        "sort_by" => {
            argc(&args, 0, 0)?;
            let b = block_or_enum!(blk, recv, name, args);
            Ok(Value::array(sort_by(it, items, b)?))
        }
        "uniq" => {
            argc(&args, 0, 0)?;
            Ok(Value::array(hkey_uniq(it, items, blk)?))
        }
        "to_a" | "entries" => {
            argc(&args, 0, 0)?;
            Ok(Value::array(items))
        }
        "to_h" => {
            argc(&args, 0, 0)?;
            match blk {
                Some(b) => {
                    let mut pairs = Vec::with_capacity(items.len());
                    for x in items {
                        pairs.push(it.yield_keep(b, x)?);
                    }
                    pairs_to_hash(&pairs)
                }
                None => pairs_to_hash(&items),
            }
        }
        _ => {
            if super::OBJECT_METHODS.contains(&name) {
                object::call(it, recv, name, args, blk)
            } else {
                no_method(recv, name)
            }
        }
    }
}

fn sym_name(v: &Value) -> R<Arc<str>> {
    match v {
        Value::Sym(s) | Value::Str(s) => Ok(s.clone()),
        other => fail(TYPE_ERROR, format!("{} is not a symbol nor a string", other.inspect()?)),
    }
}
