//! Hash methods (§8.9). Hashes keep insertion order and use `eql?` keys.

use super::array::dig;
use super::enumerable::{self, pairs_to_hash};
use super::{argc, block_or_enum, object};
use crate::error::{fail, R, KEY_ERROR, TYPE_ERROR};
use crate::interp::{BlockArg, Interp};
use crate::value::{HashCell, RHash, Value};
use std::rc::Rc;

fn pair(k: &Value, v: &Value) -> Value {
    Value::array(vec![k.clone(), v.clone()])
}

fn to_hash(v: &Value) -> R<RHash> {
    match v {
        Value::Hash(h) => Ok(h.borrow().clone()),
        other => fail(TYPE_ERROR, format!("no implicit conversion of {} into Hash", other.type_desc())),
    }
}

fn key_error(k: &Value) -> R<Value> {
    fail(KEY_ERROR, format!("key not found: {}", k.inspect()?))
}

fn merge_into(it: &mut Interp, target: &mut RHash, others: &[Value], blk: Option<&BlockArg>) -> R<()> {
    for o in others {
        for (k, v) in to_hash(o)?.pairs() {
            let nv = match (target.get(&k), blk) {
                (Some(old), Some(b)) => it.call_block(b, vec![k.clone(), old, v])?,
                _ => v,
            };
            target.insert(k, nv);
        }
    }
    Ok(())
}

pub fn call(it: &mut Interp, recv: &Value, name: &str, args: Vec<Value>, blk: Option<&BlockArg>) -> R<Value> {
    let Value::Hash(h) = recv else { return object::call(it, recv, name, args, blk) };
    let h: &Rc<HashCell> = h;
    const MUTATORS: &[&str] = &["[]=", "store", "delete", "delete_if", "clear", "merge!", "update", "default="];
    if h.frozen.get() && MUTATORS.contains(&name) {
        return fail(crate::error::FROZEN_ERROR, format!("can't modify frozen Hash: {}", recv.inspect()?));
    }
    let pairs = || h.borrow().pairs();
    match name {
        "[]" => {
            argc(&args, 1, 1)?;
            let hb = h.borrow();
            Ok(hb.get(&args[0]).unwrap_or_else(|| hb.default.clone()))
        }
        "[]=" | "store" => {
            argc(&args, 2, 2)?;
            super::check_array_len(h.borrow().len() + 1)?;
            h.borrow_mut().insert(args[0].clone(), args[1].clone());
            Ok(args[1].clone())
        }
        "fetch" => {
            argc(&args, 1, 2)?;
            let found = h.borrow().get(&args[0]);
            match found {
                Some(v) => Ok(v),
                None => match (blk, args.get(1)) {
                    (Some(b), _) => it.yield1(b, args[0].clone()),
                    (None, Some(d)) => Ok(d.clone()),
                    _ => key_error(&args[0]),
                },
            }
        }
        "dig" => dig(it, recv, &args),
        "key?" | "has_key?" | "include?" | "member?" => {
            argc(&args, 1, 1)?;
            Ok(Value::Bool(h.borrow().contains(&args[0])))
        }
        "value?" | "has_value?" => {
            argc(&args, 1, 1)?;
            Ok(Value::Bool(h.borrow().iter().any(|(_, v)| v.equals(&args[0]))))
        }
        "key" => {
            argc(&args, 1, 1)?;
            Ok(h.borrow().iter().find(|(_, v)| v.equals(&args[0])).map(|(k, _)| k.clone()).unwrap_or(Value::Nil))
        }
        "keys" => {
            argc(&args, 0, 0)?;
            Ok(Value::array(h.borrow().keys()))
        }
        "values" => {
            argc(&args, 0, 0)?;
            Ok(Value::array(h.borrow().values()))
        }
        "values_at" => {
            let hb = h.borrow();
            Ok(Value::array(args.iter().map(|k| hb.get(k).unwrap_or_else(|| hb.default.clone())).collect()))
        }
        "fetch_values" => {
            let mut out = Vec::new();
            for k in &args {
                let found = h.borrow().get(k);
                out.push(match (found, blk) {
                    (Some(v), _) => v,
                    (None, Some(b)) => it.yield1(b, k.clone())?,
                    (None, None) => return key_error(k),
                });
            }
            Ok(Value::array(out))
        }
        "length" | "size" => {
            argc(&args, 0, 0)?;
            Ok(Value::Int(h.borrow().len() as i64))
        }
        "empty?" => {
            argc(&args, 0, 0)?;
            Ok(Value::Bool(h.borrow().is_empty()))
        }
        "delete" => {
            argc(&args, 1, 1)?;
            let removed = h.borrow_mut().remove(&args[0]);
            match (removed, blk) {
                (Some(v), _) => Ok(v),
                (None, Some(b)) => it.yield1(b, args[0].clone()),
                (None, None) => Ok(Value::Nil),
            }
        }
        "delete_if" => {
            argc(&args, 0, 0)?;
            let b = block_or_enum!(blk, recv, name, args);
            for (k, v) in pairs() {
                if it.call_block(b, vec![k.clone(), v])?.truthy() {
                    h.borrow_mut().remove(&k);
                }
            }
            Ok(recv.clone())
        }
        "clear" => {
            argc(&args, 0, 0)?;
            h.borrow_mut().clear();
            Ok(recv.clone())
        }
        "merge" => {
            let mut target = h.borrow().clone();
            merge_into(it, &mut target, &args, blk)?;
            Ok(Value::hash(target))
        }
        "merge!" | "update" => {
            let mut target = h.borrow().clone();
            merge_into(it, &mut target, &args, blk)?;
            *h.borrow_mut() = target;
            Ok(recv.clone())
        }
        "to_a" => {
            argc(&args, 0, 0)?;
            Ok(Value::array(pairs().iter().map(|(k, v)| pair(k, v)).collect()))
        }
        "to_h" => {
            argc(&args, 0, 0)?;
            let Some(b) = blk else { return Ok(recv.clone()) };
            let mut out = Vec::new();
            for (k, v) in pairs() {
                out.push(it.call_block(b, vec![k, v])?);
            }
            pairs_to_hash(&out)
        }
        "invert" => {
            argc(&args, 0, 0)?;
            Ok(Value::hash_from(pairs().into_iter().map(|(k, v)| (v, k)).collect()))
        }
        "transform_values" | "transform_keys" => {
            argc(&args, 0, 1)?;
            if name == "transform_keys" {
                if let Some(Value::Hash(map)) = args.first() {
                    let map = map.borrow().clone();
                    let mut out = RHash::new();
                    for (k, v) in pairs() {
                        let nk = match (map.get(&k), blk) {
                            (Some(nk), _) => nk,
                            (None, Some(b)) => it.yield1(b, k)?,
                            (None, None) => k,
                        };
                        out.insert(nk, v);
                    }
                    return Ok(Value::hash(out));
                }
            }
            argc(&args, 0, 0)?;
            let b = block_or_enum!(blk, recv, name, args);
            let mut out = RHash::new();
            for (k, v) in pairs() {
                if name == "transform_values" {
                    let nv = it.yield_keep(b, v)?;
                    out.insert(k, nv);
                } else {
                    let nk = it.yield1(b, k)?;
                    out.insert(nk, v);
                }
            }
            Ok(Value::hash(out))
        }
        "select" | "filter" | "reject" => {
            argc(&args, 0, 0)?;
            let b = block_or_enum!(blk, recv, name, args);
            let keep = name != "reject";
            let mut out = RHash::new();
            for (k, v) in pairs() {
                if it.call_block(b, vec![k.clone(), v.clone()])?.truthy() == keep {
                    out.insert(k, v);
                }
            }
            Ok(Value::hash(out))
        }
        "filter_map" => {
            argc(&args, 0, 0)?;
            let b = block_or_enum!(blk, recv, name, args);
            let mut out = Vec::new();
            for (k, v) in pairs() {
                let r = it.yield1(b, pair(&k, &v))?;
                if r.truthy() {
                    out.push(r);
                }
            }
            Ok(Value::array(out))
        }
        "each" | "each_pair" => {
            argc(&args, 0, 0)?;
            let b = block_or_enum!(blk, recv, name, args);
            for (k, v) in pairs() {
                it.yield1(b, pair(&k, &v))?;
            }
            Ok(recv.clone())
        }
        "each_key" | "each_value" => {
            argc(&args, 0, 0)?;
            let b = block_or_enum!(blk, recv, name, args);
            for (k, v) in pairs() {
                it.yield1(b, if name == "each_key" { k } else { v })?;
            }
            Ok(recv.clone())
        }
        "slice" => {
            let hb = h.borrow();
            let mut out = RHash::new();
            for k in &args {
                if let Some(v) = hb.get(k) {
                    out.insert(k.clone(), v);
                }
            }
            Ok(Value::hash(out))
        }
        "except" => {
            let mut out = h.borrow().clone();
            out.default = Value::Nil;
            for k in &args {
                out.remove(k);
            }
            Ok(Value::hash(out))
        }
        "compact" => {
            argc(&args, 0, 0)?;
            Ok(Value::hash_from(pairs().into_iter().filter(|(_, v)| !v.is_nil()).collect()))
        }
        "default" => {
            argc(&args, 0, 1)?;
            Ok(h.borrow().default.clone())
        }
        "default=" => {
            argc(&args, 1, 1)?;
            h.borrow_mut().default = args[0].clone();
            Ok(args[0].clone())
        }
        "first" => {
            argc(&args, 0, 1)?;
            if args.is_empty() {
                return Ok(h.borrow().first().map(|(k, v)| pair(&k, &v)).unwrap_or(Value::Nil));
            }
            let items: Vec<Value> = pairs().iter().map(|(k, v)| pair(k, v)).collect();
            enumerable::call(it, recv, items, name, args, blk)
        }
        _ => {
            if super::HASH_METHODS.contains(&name) && !super::OBJECT_METHODS.contains(&name) {
                let items: Vec<Value> = pairs().iter().map(|(k, v)| pair(k, v)).collect();
                return enumerable::call(it, recv, items, name, args, blk);
            }
            object::call(it, recv, name, args, blk)
        }
    }
}
