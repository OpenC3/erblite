//! Methods every value has (§8.1), plus nil and boolean methods (§8.2).

use super::{argc, block_or_enum, no_method, range, responds};
use crate::error::{fail, R, ARGUMENT_ERROR, TYPE_ERROR};
use crate::interp::{BlockArg, Interp};
use crate::value::{RHash, Value};

/// Ruby `===` (used by `case` and pattern arguments).
pub fn case_eq(pat: &Value, v: &Value) -> bool {
    match pat {
        Value::Class(c) => v.class_id().ancestors().contains(c),
        Value::Range(r) => range::cover(r, v),
        _ => pat.equals(v),
    }
}

fn class_arg(args: &[Value]) -> R<crate::value::ClassId> {
    match args.first() {
        Some(Value::Class(c)) => Ok(*c),
        _ => fail(TYPE_ERROR, "class or module required"),
    }
}

pub fn call(it: &mut Interp, recv: &Value, name: &str, args: Vec<Value>, blk: Option<&BlockArg>) -> R<Value> {
    match name {
        "==" => {
            argc(&args, 1, 1)?;
            Ok(Value::Bool(recv.equals(&args[0])))
        }
        "!=" => {
            argc(&args, 1, 1)?;
            Ok(Value::Bool(!recv.equals(&args[0])))
        }
        "!" => {
            argc(&args, 0, 0)?;
            Ok(Value::Bool(!recv.truthy()))
        }
        "===" => {
            argc(&args, 1, 1)?;
            Ok(Value::Bool(case_eq(recv, &args[0])))
        }
        "eql?" => {
            argc(&args, 1, 1)?;
            Ok(Value::Bool(recv.eql(&args[0])))
        }
        "<=>" => {
            argc(&args, 1, 1)?;
            Ok(match recv.compare(&args[0]) {
                Some(o) => Value::Int(o as i64),
                None => Value::Nil,
            })
        }
        "nil?" => {
            argc(&args, 0, 0)?;
            Ok(Value::Bool(recv.is_nil()))
        }
        "to_s" => {
            argc(&args, 0, 0)?;
            Ok(match recv {
                Value::Str(_) => recv.clone(),
                _ => Value::string(recv.to_s()?),
            })
        }
        "inspect" => {
            argc(&args, 0, 0)?;
            Ok(Value::string(recv.inspect()?))
        }
        "class" => {
            argc(&args, 0, 0)?;
            Ok(Value::Class(recv.class_id()))
        }
        "is_a?" | "kind_of?" => {
            argc(&args, 1, 1)?;
            let c = class_arg(&args)?;
            Ok(Value::Bool(recv.class_id().ancestors().contains(&c)))
        }
        "instance_of?" => {
            argc(&args, 1, 1)?;
            let c = class_arg(&args)?;
            Ok(Value::Bool(recv.class_id() == c))
        }
        "respond_to?" => {
            argc(&args, 1, 2)?;
            let n = match &args[0] {
                Value::Sym(s) | Value::Str(s) => s.clone(),
                other => return fail(TYPE_ERROR, format!("{} is not a symbol nor a string", other.inspect()?)),
            };
            Ok(Value::Bool(responds(recv, &n)))
        }
        "dup" => {
            argc(&args, 0, 0)?;
            Ok(match recv {
                Value::Array(a) => Value::array(a.borrow().clone()),
                Value::Hash(h) => Value::hash(h.borrow().clone()),
                Value::Str(s) => Value::str(s),
                other => other.clone(),
            })
        }
        "freeze" => {
            argc(&args, 0, 0)?;
            Ok(match recv {
                Value::Str(s) if !it.ctx.is_frozen_str(s) => Value::Str(it.ctx.freeze_str(s)),
                Value::Array(a) => {
                    a.frozen.set(true);
                    recv.clone()
                }
                Value::Hash(h) => {
                    h.frozen.set(true);
                    recv.clone()
                }
                other => other.clone(),
            })
        }
        "frozen?" => {
            argc(&args, 0, 0)?;
            Ok(Value::Bool(match recv {
                Value::Str(s) => it.ctx.is_frozen_str(s),
                other => other.is_frozen(),
            }))
        }
        "to_json" => {
            argc(&args, 0, 1)?;
            Ok(Value::string(crate::json::generate(recv)?))
        }
        "then" => {
            argc(&args, 0, 0)?;
            let b = block_or_enum!(blk, recv, name, args);
            it.yield1(b, recv.clone())
        }
        "tap" => {
            argc(&args, 0, 0)?;
            let Some(b) = blk else {
                return fail(ARGUMENT_ERROR, "no block given (tap)");
            };
            it.yield1(b, recv.clone())?;
            Ok(recv.clone())
        }
        _ => no_method(recv, name),
    }
}

/// nil and boolean specific methods.
pub fn call_nil_bool(it: &mut Interp, recv: &Value, name: &str, args: Vec<Value>, blk: Option<&BlockArg>) -> R<Value> {
    match (recv, name) {
        (Value::Nil, "to_a") => {
            argc(&args, 0, 0)?;
            Ok(Value::array(vec![]))
        }
        (Value::Nil, "to_i") => {
            argc(&args, 0, 0)?;
            Ok(Value::Int(0))
        }
        (Value::Nil, "to_f") => {
            argc(&args, 0, 0)?;
            Ok(Value::Float(0.0))
        }
        (Value::Nil, "to_h") => {
            argc(&args, 0, 0)?;
            Ok(Value::hash(RHash::new()))
        }
        (Value::Bool(b), "&" | "|" | "^") => {
            argc(&args, 1, 1)?;
            let o = args[0].truthy();
            Ok(Value::Bool(match name {
                "&" => *b && o,
                "|" => *b || o,
                _ => *b != o,
            }))
        }
        _ => call(it, recv, name, args, blk),
    }
}
