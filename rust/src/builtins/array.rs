//! Array methods (§8.7). Arrays are mutable and shared by reference.

use std::sync::Arc;
use super::enumerable::{self, sort_by, sort_values, sum_values, uniq};
use super::{argc, block_or_enum, check_array_len, check_str_len, object, range, to_int, to_str};
use crate::error::{fail, R, ARGUMENT_ERROR, INDEX_ERROR, LIMIT_ERROR, RANGE_ERROR, TYPE_ERROR};
use crate::interp::{BlockArg, Interp};
use crate::value::{Arr, Value, MAX_DATA_DEPTH};
use std::collections::HashSet;
use std::rc::Rc;

fn to_ary(v: &Value) -> R<Vec<Value>> {
    match v {
        Value::Array(a) => Ok(a.borrow().clone()),
        other => fail(TYPE_ERROR, format!("no implicit conversion of {} into Array", other.type_desc())),
    }
}

fn index_get(items: &[Value], args: &[Value]) -> R<Value> {
    argc(args, 1, 2)?;
    let n = items.len() as i64;
    if args.len() == 2 {
        let start = to_int(&args[0])?;
        let len = to_int(&args[1])?;
        let st = if start < 0 { start + n } else { start };
        if st < 0 || st > n || len < 0 {
            return Ok(Value::Nil);
        }
        let e = st.saturating_add(len).min(n);
        return Ok(Value::array(items[st as usize..e as usize].to_vec()));
    }
    match &args[0] {
        Value::Range(r) => match range::beg_len(r, n)? {
            Some((b, l)) => Ok(Value::array(items[b as usize..(b + l) as usize].to_vec())),
            None => Ok(Value::Nil),
        },
        other => {
            let i = to_int(other)?;
            let i = if i < 0 { i + n } else { i };
            Ok(if i < 0 || i >= n { Value::Nil } else { items[i as usize].clone() })
        }
    }
}

/// Replaces `len` items at `start` with `rep`, padding with nil past the end.
fn splice(a: &Rc<Arr>, start: i64, len: i64, rep: Vec<Value>) -> R<()> {
    let mut v = a.borrow_mut();
    let n = v.len() as i64;
    let start = start as usize;
    if start as i64 > n {
        check_array_len(start + rep.len())?;
        v.resize(start, Value::Nil);
    }
    let n = v.len();
    let end = (start + len.max(0) as usize).min(n);
    check_array_len(n - (end - start) + rep.len())?;
    v.splice(start..end, rep);
    Ok(())
}

fn index_set(a: &Rc<Arr>, args: &[Value]) -> R<Value> {
    argc(args, 2, 3)?;
    let n = a.borrow().len() as i64;
    let val = args[args.len() - 1].clone();
    let as_list = |v: &Value| match v {
        Value::Array(x) => x.borrow().clone(),
        other => vec![other.clone()],
    };
    if args.len() == 3 {
        let mut start = to_int(&args[0])?;
        let len = to_int(&args[1])?;
        if start < 0 {
            start += n;
            if start < 0 {
                return fail(INDEX_ERROR, format!("index {} too small for array; minimum: -{}", start - n, n));
            }
        }
        if len < 0 {
            return fail(INDEX_ERROR, format!("negative length ({})", len));
        }
        splice(a, start, len, as_list(&val))?;
        return Ok(val);
    }
    match &args[0] {
        Value::Range(r) => {
            let mut beg = match &r.start {
                Value::Nil => 0,
                v => to_int(v)?,
            };
            let mut end = match &r.end {
                Value::Nil => n,
                v => to_int(v)?,
            };
            if beg < 0 {
                beg += n;
                if beg < 0 {
                    return fail(RANGE_ERROR, format!("{} out of range", args[0].inspect()?));
                }
            }
            if end < 0 {
                end += n;
            }
            if !r.excl && !r.end.is_nil() {
                end += 1;
            }
            let len = (end - beg).max(0);
            splice(a, beg, len, as_list(&val))?;
            Ok(val)
        }
        other => {
            let mut i = to_int(other)?;
            if i < 0 {
                i += n;
                if i < 0 {
                    return fail(INDEX_ERROR, format!("index {} too small for array; minimum: -{}", i - n, n));
                }
            }
            let iu = i as usize;
            check_array_len(iu + 1)?;
            let mut v = a.borrow_mut();
            if iu >= v.len() {
                v.resize(iu, Value::Nil);
                v.push(val.clone());
            } else {
                v[iu] = val.clone();
            }
            Ok(val)
        }
    }
}

/// `Array#join`, recursing into nested arrays.
fn join_into(items: &[Value], sep: &str, out: &mut String, seen: &mut Vec<usize>) -> R<()> {
    if seen.len() > MAX_DATA_DEPTH || crate::stack::exhausted() {
        return fail(LIMIT_ERROR, "nesting too deep to join");
    }
    for (i, x) in items.iter().enumerate() {
        if i > 0 {
            out.push_str(sep);
        }
        match x {
            Value::Array(a) => {
                let id = Rc::as_ptr(a) as *const u8 as usize;
                if seen.contains(&id) {
                    return fail(ARGUMENT_ERROR, "recursive array join");
                }
                seen.push(id);
                let inner = a.borrow().clone();
                join_into(&inner, sep, out, seen)?;
                seen.pop();
            }
            other => out.push_str(&other.to_s()?),
        }
        check_str_len(out.len())?;
    }
    Ok(())
}

pub fn join(items: &[Value], sep: &str) -> R<String> {
    let mut out = String::new();
    join_into(items, sep, &mut out, &mut Vec::new())?;
    Ok(out)
}

fn flatten_into(items: &[Value], depth: i64, out: &mut Vec<Value>, seen: &mut Vec<usize>) -> R<()> {
    if seen.len() > MAX_DATA_DEPTH || crate::stack::exhausted() {
        return fail(LIMIT_ERROR, "nesting too deep to flatten");
    }
    for x in items {
        match x {
            Value::Array(a) if depth != 0 => {
                let id = Rc::as_ptr(a) as *const u8 as usize;
                if seen.contains(&id) {
                    return fail(ARGUMENT_ERROR, "tried to flatten recursive array");
                }
                seen.push(id);
                let inner = a.borrow().clone();
                flatten_into(&inner, depth - 1, out, seen)?;
                seen.pop();
            }
            other => {
                out.push(other.clone());
                check_array_len(out.len())?;
            }
        }
    }
    Ok(())
}

/// `dig` through arrays and hashes.
pub fn dig(it: &mut Interp, recv: &Value, keys: &[Value]) -> R<Value> {
    argc(keys, 1, usize::MAX)?;
    let mut cur = recv.clone();
    for (i, k) in keys.iter().enumerate() {
        cur = match &cur {
            Value::Array(_) | Value::Hash(_) => it.call_method(&cur, "[]", vec![k.clone()], None)?,
            Value::Nil => return Ok(Value::Nil),
            other if i > 0 => return fail(TYPE_ERROR, format!("{} does not have #dig method", other.class_name())),
            other => return super::no_method(other, "dig"),
        };
    }
    Ok(cur)
}

fn combinations(len: usize, k: usize, f: &mut dyn FnMut(&[usize]) -> R<()>) -> R<()> {
    if k > len {
        return Ok(());
    }
    let mut idx: Vec<usize> = (0..k).collect();
    loop {
        f(&idx)?;
        // rightmost position that can still move right
        let mut i = k;
        loop {
            if i == 0 {
                return Ok(());
            }
            i -= 1;
            if idx[i] < len - k + i {
                break;
            }
        }
        idx[i] += 1;
        for j in i + 1..k {
            idx[j] = idx[j - 1] + 1;
        }
    }
}

fn permutations(len: usize, k: usize, f: &mut dyn FnMut(&[usize]) -> R<()>) -> R<()> {
    if k > len {
        return Ok(());
    }
    let mut used = vec![false; len];
    let mut chosen: Vec<usize> = Vec::with_capacity(k);
    let mut stack: Vec<usize> = vec![0];
    loop {
        if chosen.len() == k {
            f(&chosen)?;
            stack.pop();
            if let Some(last) = chosen.pop() {
                used[last] = false;
            }
            if stack.is_empty() {
                return Ok(());
            }
            continue;
        }
        let level = stack.len() - 1;
        let mut c = stack[level];
        while c < len && used[c] {
            c += 1;
        }
        if c >= len {
            stack.pop();
            if let Some(last) = chosen.pop() {
                used[last] = false;
            }
            if stack.is_empty() {
                return Ok(());
            }
            continue;
        }
        stack[level] = c + 1;
        used[c] = true;
        chosen.push(c);
        stack.push(0);
    }
}

fn hkeys(items: &[Value]) -> HashSet<crate::value::HKey> {
    items.iter().map(|x| x.hkey()).collect()
}

fn opt_count(args: &[Value]) -> R<Option<usize>> {
    argc(args, 0, 1)?;
    match args.first() {
        None => Ok(None),
        Some(v) => {
            let n = to_int(v)?;
            if n < 0 {
                return fail(ARGUMENT_ERROR, "negative array size");
            }
            Ok(Some(n as usize))
        }
    }
}

pub fn call(it: &mut Interp, recv: &Value, name: &str, args: Vec<Value>, blk: Option<&BlockArg>) -> R<Value> {
    let Value::Array(a) = recv else { return object::call(it, recv, name, args, blk) };
    const MUTATORS: &[&str] = &[
        "[]=", "<<", "push", "append", "pop", "shift", "unshift", "prepend", "insert", "concat", "delete", "delete_at",
        "delete_if", "clear", "fill",
    ];
    if a.frozen.get() && MUTATORS.contains(&name) {
        return fail(crate::error::FROZEN_ERROR, format!("can't modify frozen Array: {}", recv.inspect()?));
    }
    let snapshot = || a.borrow().clone();
    match name {
        "[]" | "slice" => index_get(&a.borrow(), &args),
        "[]=" => index_set(a, &args),
        "+" => {
            argc(&args, 1, 1)?;
            let mut v = snapshot();
            let o = to_ary(&args[0])?;
            check_array_len(v.len() + o.len())?;
            v.extend(o);
            Ok(Value::array(v))
        }
        "-" => {
            argc(&args, 1, 1)?;
            let o = hkeys(&to_ary(&args[0])?);
            Ok(Value::array(snapshot().into_iter().filter(|x| !o.contains(&x.hkey())).collect()))
        }
        "*" => {
            argc(&args, 1, 1)?;
            if let Value::Str(sep) = &args[0] {
                return Ok(Value::string(join(&snapshot(), sep)?));
            }
            let n = to_int(&args[0])?;
            if n < 0 {
                return fail(ARGUMENT_ERROR, "negative argument");
            }
            let v = snapshot();
            check_array_len(v.len().saturating_mul(n as usize))?;
            let mut out = Vec::with_capacity(v.len() * n as usize);
            for _ in 0..n {
                out.extend_from_slice(&v);
            }
            Ok(Value::array(out))
        }
        "&" => {
            argc(&args, 1, 1)?;
            let o = hkeys(&to_ary(&args[0])?);
            let mut seen = HashSet::new();
            Ok(Value::array(
                snapshot().into_iter().filter(|x| o.contains(&x.hkey()) && seen.insert(x.hkey())).collect(),
            ))
        }
        "|" => {
            argc(&args, 1, 1)?;
            let mut v = snapshot();
            v.extend(to_ary(&args[0])?);
            Ok(Value::array(uniq(it, v, None)?))
        }
        "<<" | "push" | "append" => {
            if name == "<<" {
                argc(&args, 1, 1)?;
            }
            let mut v = a.borrow_mut();
            check_array_len(v.len() + args.len())?;
            v.extend(args);
            Ok(recv.clone())
        }
        "<=>" => {
            argc(&args, 1, 1)?;
            Ok(match recv.compare(&args[0]) {
                Some(o) => Value::Int(o as i64),
                None => Value::Nil,
            })
        }
        "length" | "size" => {
            argc(&args, 0, 0)?;
            Ok(Value::Int(a.borrow().len() as i64))
        }
        "empty?" => {
            argc(&args, 0, 0)?;
            Ok(Value::Bool(a.borrow().is_empty()))
        }
        "first" | "last" => {
            let v = a.borrow();
            match opt_count(&args)? {
                None => Ok((if name == "first" { v.first() } else { v.last() }).cloned().unwrap_or(Value::Nil)),
                Some(n) => {
                    let k = n.min(v.len());
                    Ok(Value::array(if name == "first" { v[..k].to_vec() } else { v[v.len() - k..].to_vec() }))
                }
            }
        }
        "pop" | "shift" => {
            let mut v = a.borrow_mut();
            match opt_count(&args)? {
                None => Ok(if v.is_empty() {
                    Value::Nil
                } else if name == "pop" {
                    v.pop().unwrap_or(Value::Nil)
                } else {
                    v.remove(0)
                }),
                Some(n) => {
                    let k = n.min(v.len());
                    let out: Vec<Value> = if name == "pop" {
                        let at = v.len() - k;
                        v.split_off(at)
                    } else {
                        v.drain(..k).collect()
                    };
                    Ok(Value::array(out))
                }
            }
        }
        "unshift" | "prepend" => {
            let mut v = a.borrow_mut();
            check_array_len(v.len() + args.len())?;
            v.splice(0..0, args);
            Ok(recv.clone())
        }
        "insert" => {
            argc(&args, 1, usize::MAX)?;
            let mut i = to_int(&args[0])?;
            if args.len() == 1 {
                return Ok(recv.clone());
            }
            let n = a.borrow().len() as i64;
            if i < 0 {
                i += n + 1;
                if i < 0 {
                    return fail(INDEX_ERROR, format!("index {} too small for array; minimum: -{}", i - n - 1, n + 1));
                }
            }
            splice(a, i, 0, args[1..].to_vec())?;
            Ok(recv.clone())
        }
        "concat" => {
            let mut extra = Vec::new();
            for x in &args {
                extra.extend(to_ary(x)?);
            }
            let mut v = a.borrow_mut();
            check_array_len(v.len() + extra.len())?;
            v.extend(extra);
            Ok(recv.clone())
        }
        "delete" => {
            argc(&args, 1, 1)?;
            let mut v = a.borrow_mut();
            let before = v.len();
            v.retain(|x| !x.equals(&args[0]));
            Ok(if v.len() != before { args[0].clone() } else { Value::Nil })
        }
        "delete_at" => {
            argc(&args, 1, 1)?;
            let mut v = a.borrow_mut();
            let n = v.len() as i64;
            let i = to_int(&args[0])?;
            let i = if i < 0 { i + n } else { i };
            Ok(if i < 0 || i >= n { Value::Nil } else { v.remove(i as usize) })
        }
        "delete_if" => {
            argc(&args, 0, 0)?;
            let b = block_or_enum!(blk, recv, name, args);
            let mut keep = Vec::new();
            for x in snapshot() {
                if !it.yield1(b, x.clone())?.truthy() {
                    keep.push(x);
                }
            }
            *a.borrow_mut() = keep;
            Ok(recv.clone())
        }
        "clear" => {
            argc(&args, 0, 0)?;
            a.borrow_mut().clear();
            Ok(recv.clone())
        }
        "fill" => {
            if let Some(b) = blk {
                argc(&args, 0, 0)?;
                let n = a.borrow().len();
                for i in 0..n {
                    let v = it.yield1(b, Value::Int(i as i64))?;
                    if let Some(slot) = a.borrow_mut().get_mut(i) {
                        *slot = v;
                    }
                }
                return Ok(recv.clone());
            }
            argc(&args, 1, 1)?;
            for slot in a.borrow_mut().iter_mut() {
                *slot = args[0].clone();
            }
            Ok(recv.clone())
        }
        "index" | "find_index" | "rindex" => {
            argc(&args, 0, 1)?;
            let mut v: Vec<(usize, Value)> = snapshot().into_iter().enumerate().collect();
            if name == "rindex" {
                v.reverse();
            }
            if let Some(t) = args.first() {
                return Ok(v.iter().find(|(_, x)| x.equals(t)).map(|(i, _)| Value::Int(*i as i64)).unwrap_or(Value::Nil));
            }
            let b = block_or_enum!(blk, recv, name, args);
            for (i, x) in v {
                if it.yield1(b, x)?.truthy() {
                    return Ok(Value::Int(i as i64));
                }
            }
            Ok(Value::Nil)
        }
        "join" => {
            argc(&args, 0, 1)?;
            let sep = match args.first() {
                None | Some(Value::Nil) => Arc::from(""),
                Some(v) => to_str(v)?,
            };
            Ok(Value::string(join(&snapshot(), &sep)?))
        }
        "reverse" => {
            argc(&args, 0, 0)?;
            let mut v = snapshot();
            v.reverse();
            Ok(Value::array(v))
        }
        "rotate" => {
            argc(&args, 0, 1)?;
            let n = match args.first() {
                Some(x) => to_int(x)?,
                None => 1,
            };
            let mut v = snapshot();
            if !v.is_empty() {
                let k = n.rem_euclid(v.len() as i64) as usize;
                v.rotate_left(k);
            }
            Ok(Value::array(v))
        }
        "sort" => {
            argc(&args, 0, 0)?;
            Ok(Value::array(sort_values(it, snapshot(), blk)?))
        }
        "sort_by" => {
            argc(&args, 0, 0)?;
            let b = block_or_enum!(blk, recv, name, args);
            Ok(Value::array(sort_by(it, snapshot(), b)?))
        }
        "uniq" => {
            argc(&args, 0, 0)?;
            Ok(Value::array(uniq(it, snapshot(), blk)?))
        }
        "flatten" => {
            argc(&args, 0, 1)?;
            let depth = match args.first() {
                None | Some(Value::Nil) => -1,
                Some(d) => to_int(d)?,
            };
            let mut out = Vec::new();
            let id = Rc::as_ptr(a) as *const u8 as usize;
            flatten_into(&snapshot(), depth, &mut out, &mut vec![id])?;
            Ok(Value::array(out))
        }
        "compact" => {
            argc(&args, 0, 0)?;
            Ok(Value::array(snapshot().into_iter().filter(|x| !x.is_nil()).collect()))
        }
        "transpose" => {
            argc(&args, 0, 0)?;
            let rows: Vec<Vec<Value>> = snapshot().iter().map(to_ary).collect::<R<_>>()?;
            let Some(first) = rows.first() else { return Ok(Value::array(vec![])) };
            let w = first.len();
            for r in &rows {
                if r.len() != w {
                    return fail(INDEX_ERROR, format!("element size differs ({} should be {})", r.len(), w));
                }
            }
            Ok(Value::array(
                (0..w).map(|c| Value::array(rows.iter().map(|r| r[c].clone()).collect())).collect(),
            ))
        }
        "values_at" => {
            let v = snapshot();
            let mut out = Vec::new();
            for x in &args {
                match index_get(&v, std::slice::from_ref(x))? {
                    Value::Array(sub) if matches!(x, Value::Range(_)) => out.extend(sub.borrow().iter().cloned()),
                    other => out.push(other),
                }
            }
            Ok(Value::array(out))
        }
        "dig" => dig(it, recv, &args),
        "assoc" => {
            argc(&args, 1, 1)?;
            for x in snapshot() {
                if let Value::Array(p) = &x {
                    if p.borrow().first().is_some_and(|k| k.equals(&args[0])) {
                        return Ok(x.clone());
                    }
                }
            }
            Ok(Value::Nil)
        }
        "product" => {
            let mut lists = vec![snapshot()];
            for x in &args {
                lists.push(to_ary(x)?);
            }
            let total = lists.iter().try_fold(1usize, |acc, l| acc.checked_mul(l.len()));
            match total {
                Some(t) if t <= super::MAX_ARRAY_LEN => {}
                _ => return fail(LIMIT_ERROR, "product too large"),
            }
            let mut out = vec![Vec::new()];
            for l in &lists {
                let mut next = Vec::new();
                for prefix in &out {
                    for x in l {
                        let mut p = prefix.clone();
                        p.push(x.clone());
                        next.push(p);
                    }
                }
                out = next;
            }
            let rows: Vec<Value> = out.into_iter().map(Value::array).collect();
            if let Some(b) = blk {
                for r in rows {
                    it.yield1(b, r)?;
                }
                return Ok(recv.clone());
            }
            Ok(Value::array(rows))
        }
        "combination" | "permutation" => {
            let v = snapshot();
            let k = if name == "permutation" && args.is_empty() {
                v.len() as i64
            } else {
                argc(&args, 1, 1)?;
                to_int(&args[0])?
            };
            let mut out = Vec::new();
            if k >= 0 {
                let mut f = |idx: &[usize]| -> R<()> {
                    let combo = Value::array(idx.iter().map(|&i| v[i].clone()).collect());
                    match blk {
                        Some(b) => {
                            it.yield1(b, combo)?;
                        }
                        None => {
                            out.push(combo);
                            check_array_len(out.len())?;
                        }
                    }
                    Ok(())
                };
                if name == "combination" {
                    combinations(v.len(), k as usize, &mut f)?;
                } else {
                    permutations(v.len(), k as usize, &mut f)?;
                }
            }
            if blk.is_some() {
                return Ok(recv.clone());
            }
            Ok(Value::array(out))
        }
        "to_a" => {
            argc(&args, 0, 0)?;
            Ok(recv.clone())
        }
        "each" => {
            argc(&args, 0, 0)?;
            let b = block_or_enum!(blk, recv, name, args);
            let mut i = 0;
            loop {
                let item = match a.borrow().get(i) {
                    Some(v) => v.clone(),
                    None => break,
                };
                it.yield1(b, item)?;
                i += 1;
            }
            Ok(recv.clone())
        }
        "each_with_index" | "map" | "collect" if blk.is_some() => {
            argc(&args, 0, 0)?;
            let b = blk.ok_or_else(|| crate::error::rt_err(ARGUMENT_ERROR, "no block"))?;
            let mut i = 0;
            let mut out = Vec::new();
            loop {
                let item = match a.borrow().get(i) {
                    Some(v) => v.clone(),
                    None => break,
                };
                if name == "each_with_index" {
                    it.call_block(b, vec![item, Value::Int(i as i64)])?;
                } else {
                    out.push(it.yield_keep(b, item)?);
                }
                i += 1;
            }
            Ok(if name == "each_with_index" { recv.clone() } else { Value::array(out) })
        }
        "each_index" => {
            argc(&args, 0, 0)?;
            let b = block_or_enum!(blk, recv, name, args);
            let mut i = 0;
            while i < a.borrow().len() {
                it.yield1(b, Value::Int(i as i64))?;
                i += 1;
            }
            Ok(recv.clone())
        }
        "reverse_each" => {
            argc(&args, 0, 0)?;
            let b = block_or_enum!(blk, recv, name, args);
            for x in snapshot().into_iter().rev() {
                it.yield1(b, x)?;
            }
            Ok(recv.clone())
        }
        "sum" => {
            argc(&args, 0, 1)?;
            let init = args.first().cloned().unwrap_or(Value::Int(0));
            sum_values(it, snapshot(), init, blk, false)
        }
        "to_h" => enumerable::call(it, recv, snapshot(), name, args, blk),
        _ => {
            if super::ENUMERABLE_METHODS.contains(&name) {
                return enumerable::call(it, recv, snapshot(), name, args, blk);
            }
            object::call(it, recv, name, args, blk)
        }
    }
}
