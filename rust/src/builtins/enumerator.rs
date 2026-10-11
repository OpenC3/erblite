//! Enumerator methods (§8.7): an iterator method called without a block.

use super::{argc, block_or_enum, check_array_len, enumerable, object, pack, to_int};
use crate::error::R;
use crate::interp::{catch_break, BlockArg, Interp};
use crate::value::Value;
use std::cell::{Cell, RefCell};

/// Runs the enumerator's method and collects every yielded value.
pub fn to_vec(it: &mut Interp, v: &Value) -> R<Vec<Value>> {
    let Value::Enumerator(e) = v else { return Ok(vec![]) };
    let out = RefCell::new(Vec::new());
    let f = |_: &mut Interp, args: Vec<Value>| -> R<Value> {
        let mut o = out.borrow_mut();
        o.push(pack(args));
        check_array_len(o.len())?;
        Ok(Value::Nil)
    };
    let nb = BlockArg::Native(&f);
    catch_break(it.call_method(&e.recv, &e.meth, e.args.clone(), Some(&nb)))?;
    Ok(out.into_inner())
}

pub fn call(it: &mut Interp, recv: &Value, name: &str, args: Vec<Value>, blk: Option<&BlockArg>) -> R<Value> {
    let Value::Enumerator(e) = recv else { return object::call(it, recv, name, args, blk) };
    match name {
        "each" => {
            argc(&args, 0, 0)?;
            match blk {
                None => Ok(recv.clone()),
                Some(b) => it.call_method(&e.recv, &e.meth, e.args.clone(), Some(b)),
            }
        }
        "with_index" | "each_with_index" => {
            if name == "with_index" {
                argc(&args, 0, 1)?;
            } else {
                argc(&args, 0, 0)?;
            }
            let offset = match args.first() {
                Some(Value::Nil) | None => 0,
                Some(v) => to_int(v)?,
            };
            let ub = block_or_enum!(blk, recv, name, args);
            let idx = Cell::new(offset);
            let f = |it: &mut Interp, a: Vec<Value>| -> R<Value> {
                let i = idx.get();
                idx.set(i.wrapping_add(1));
                it.call_block(ub, vec![pack(a), Value::Int(i)])
            };
            let nb = BlockArg::Native(&f);
            it.call_method(&e.recv, &e.meth, e.args.clone(), Some(&nb))
        }
        "with_object" => {
            argc(&args, 1, 1)?;
            let ub = block_or_enum!(blk, recv, name, args);
            let memo = args[0].clone();
            let f = |it: &mut Interp, a: Vec<Value>| -> R<Value> { it.call_block(ub, vec![pack(a), memo.clone()]) };
            let nb = BlockArg::Native(&f);
            it.call_method(&e.recv, &e.meth, e.args.clone(), Some(&nb))?;
            Ok(args[0].clone())
        }
        "size" => {
            argc(&args, 0, 0)?;
            Ok(Value::Int(to_vec(it, recv)?.len() as i64))
        }
        _ => {
            if super::ENUMERABLE_METHODS.contains(&name) {
                let items = to_vec(it, recv)?;
                return enumerable::call(it, recv, items, name, args, blk);
            }
            object::call(it, recv, name, args, blk)
        }
    }
}
