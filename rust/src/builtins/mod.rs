// Copyright 2026 OpenC3, Inc.
// All Rights Reserved.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
// See LICENSE.md for more details.

//! Built-in methods. Every callable name is listed explicitly in the tables below; a name
//! that is not in the table for the receiver's type raises `NoMethodError`.

pub mod array;
pub mod conv;
pub mod enumerable;
pub mod enumerator;
pub mod file;
pub mod hash;
pub mod kernel;
pub mod numeric;
pub mod object;
pub mod range;
pub mod string;

use crate::error::{fail, ARGUMENT_ERROR, LIMIT_ERROR, NO_METHOD_ERROR, R, TYPE_ERROR};
use crate::interp::{BlockArg, Interp};
use crate::value::{ClassId, Value};
use std::sync::Arc;

/// Largest string (in bytes) a single operation may build [ERBLite size cap].
pub const MAX_STR_LEN: usize = 10_000_000;
/// Largest Array or Hash a single operation may build [ERBLite size cap].
pub const MAX_ARRAY_LEN: usize = 10_000_000;

pub const OBJECT_METHODS: &[&str] = &[
    "==",
    "!=",
    "!",
    "===",
    "eql?",
    "<=>",
    "nil?",
    "to_s",
    "inspect",
    "class",
    "is_a?",
    "kind_of?",
    "instance_of?",
    "respond_to?",
    "dup",
    "freeze",
    "frozen?",
    "to_json",
    "then",
    "tap",
];

pub const NIL_METHODS: &[&str] = &["to_a", "to_i", "to_f", "to_h"];

pub const BOOL_METHODS: &[&str] = &["&", "|", "^"];

pub const INTEGER_METHODS: &[&str] = &[
    "+",
    "-",
    "*",
    "/",
    "%",
    "**",
    "-@",
    "~",
    "&",
    "|",
    "^",
    "<<",
    ">>",
    "<=>",
    "<",
    "<=",
    ">",
    ">=",
    "abs",
    "succ",
    "next",
    "pred",
    "to_i",
    "to_f",
    "chr",
    "ord",
    "times",
    "upto",
    "downto",
    "step",
    "even?",
    "odd?",
    "zero?",
    "positive?",
    "negative?",
    "between?",
    "clamp",
    "divmod",
    "fdiv",
    "gcd",
    "lcm",
    "digits",
    "bit_length",
    "floor",
    "ceil",
    "truncate",
    "round",
];

pub const FLOAT_METHODS: &[&str] = &[
    "+",
    "-",
    "*",
    "/",
    "%",
    "**",
    "-@",
    "<=>",
    "<",
    "<=",
    ">",
    ">=",
    "abs",
    "to_f",
    "to_i",
    "truncate",
    "nan?",
    "finite?",
    "infinite?",
    "zero?",
    "positive?",
    "negative?",
    "between?",
    "clamp",
    "divmod",
    "fdiv",
    "floor",
    "ceil",
    "round",
];

pub const STRING_METHODS: &[&str] = &[
    "+",
    "*",
    "%",
    "<<",
    "<=>",
    "<",
    "<=",
    ">",
    ">=",
    "[]",
    "slice",
    "length",
    "size",
    "bytesize",
    "empty?",
    "upcase",
    "downcase",
    "capitalize",
    "swapcase",
    "strip",
    "lstrip",
    "rstrip",
    "chomp",
    "chop",
    "chars",
    "lines",
    "bytes",
    "reverse",
    "include?",
    "start_with?",
    "end_with?",
    "index",
    "rindex",
    "count",
    "sub",
    "gsub",
    "tr",
    "delete",
    "squeeze",
    "center",
    "ljust",
    "rjust",
    "to_i",
    "to_f",
    "to_str",
    "to_sym",
    "ord",
    "hex",
    "oct",
    "each_char",
    "each_line",
    "split",
    "partition",
    "rpartition",
    "casecmp",
    "casecmp?",
    "between?",
    "ascii_only?",
];

pub const SYMBOL_METHODS: &[&str] = &[
    "id2name",
    "name",
    "to_sym",
    "length",
    "size",
    "upcase",
    "downcase",
    "capitalize",
    "<=>",
    "start_with?",
    "end_with?",
    "empty?",
];

pub const ENUMERABLE_METHODS: &[&str] = &[
    "each_with_index",
    "each_with_object",
    "each_slice",
    "each_cons",
    "map",
    "collect",
    "flat_map",
    "collect_concat",
    "filter_map",
    "select",
    "filter",
    "reject",
    "find",
    "detect",
    "find_index",
    "find_all",
    "any?",
    "all?",
    "none?",
    "one?",
    "count",
    "include?",
    "member?",
    "min",
    "max",
    "min_by",
    "max_by",
    "minmax",
    "sum",
    "inject",
    "reduce",
    "group_by",
    "partition",
    "chunk_while",
    "slice_when",
    "tally",
    "zip",
    "take",
    "drop",
    "take_while",
    "drop_while",
    "first",
    "sort",
    "sort_by",
    "uniq",
    "to_a",
    "entries",
    "to_h",
];

pub const ARRAY_METHODS: &[&str] = &[
    "[]",
    "slice",
    "[]=",
    "+",
    "-",
    "*",
    "&",
    "|",
    "<<",
    "<=>",
    "length",
    "size",
    "empty?",
    "first",
    "last",
    "push",
    "append",
    "pop",
    "shift",
    "unshift",
    "prepend",
    "insert",
    "concat",
    "delete",
    "delete_at",
    "delete_if",
    "clear",
    "fill",
    "index",
    "find_index",
    "rindex",
    "include?",
    "join",
    "reverse",
    "rotate",
    "sort",
    "sort_by",
    "uniq",
    "flatten",
    "compact",
    "transpose",
    "values_at",
    "dig",
    "assoc",
    "product",
    "combination",
    "permutation",
    "to_a",
    "to_h",
    "each",
    "each_index",
    "reverse_each",
    "sum",
];

pub const HASH_METHODS: &[&str] = &[
    "[]",
    "[]=",
    "store",
    "fetch",
    "dig",
    "key?",
    "has_key?",
    "include?",
    "member?",
    "value?",
    "has_value?",
    "key",
    "keys",
    "values",
    "values_at",
    "fetch_values",
    "length",
    "size",
    "empty?",
    "delete",
    "delete_if",
    "clear",
    "merge",
    "merge!",
    "update",
    "to_a",
    "to_h",
    "invert",
    "transform_values",
    "transform_keys",
    "select",
    "filter",
    "reject",
    "filter_map",
    "min_by",
    "max_by",
    "sort_by",
    "group_by",
    "partition",
    "find",
    "detect",
    "any?",
    "all?",
    "none?",
    "count",
    "sum",
    "sort",
    "each",
    "each_pair",
    "each_key",
    "each_value",
    "each_with_index",
    "each_with_object",
    "inject",
    "reduce",
    "map",
    "collect",
    "flat_map",
    "slice",
    "except",
    "compact",
    "default",
    "default=",
    "first",
    "uniq",
    "zip",
    "tally",
];

pub const RANGE_METHODS: &[&str] = &[
    "first",
    "begin",
    "last",
    "end",
    "min",
    "max",
    "size",
    "count",
    "sum",
    "to_a",
    "entries",
    "each",
    "reverse_each",
    "step",
    "include?",
    "member?",
    "cover?",
    "exclude_end?",
];

pub const ENUMERATOR_METHODS: &[&str] = &["with_index", "each_with_index", "with_object", "each", "size"];

pub const CLASS_METHODS: &[&str] = &["name"];

pub const MATH_FUNCTIONS: &[&str] = &[
    "sqrt", "cbrt", "sin", "cos", "tan", "asin", "acos", "atan", "atan2", "exp", "log", "log2", "log10", "hypot",
];

pub const JSON_FUNCTIONS: &[&str] = &["parse", "generate", "pretty_generate"];

pub const FILE_FUNCTIONS: &[&str] = &[
    "read",
    "exist?",
    "file?",
    "directory?",
    "basename",
    "dirname",
    "extname",
    "join",
];

pub const DIR_FUNCTIONS: &[&str] = &["glob"];

/// Type-specific method table for a value.
fn type_methods(v: &Value) -> (&'static [&'static str], &'static [&'static str]) {
    const NONE: &[&str] = &[];
    match v {
        Value::Nil => (NIL_METHODS, NONE),
        Value::Bool(_) => (BOOL_METHODS, NONE),
        Value::Int(_) => (INTEGER_METHODS, NONE),
        Value::Float(_) => (FLOAT_METHODS, NONE),
        Value::Str(_) => (STRING_METHODS, NONE),
        Value::Sym(_) => (SYMBOL_METHODS, NONE),
        Value::Array(_) => (ARRAY_METHODS, ENUMERABLE_METHODS),
        Value::Hash(_) => (HASH_METHODS, NONE),
        Value::Range(_) => (RANGE_METHODS, ENUMERABLE_METHODS),
        Value::Enumerator(_) => (ENUMERATOR_METHODS, ENUMERABLE_METHODS),
        Value::Class(c) => match c {
            ClassId::Math => (MATH_FUNCTIONS, CLASS_METHODS),
            ClassId::Json => (JSON_FUNCTIONS, CLASS_METHODS),
            ClassId::File => (FILE_FUNCTIONS, CLASS_METHODS),
            ClassId::Dir => (DIR_FUNCTIONS, CLASS_METHODS),
            ClassId::Array | ClassId::Hash => (&["new"], CLASS_METHODS),
            _ => (CLASS_METHODS, NONE),
        },
    }
}

/// True when `name` is an allowed method for `v` (the table `respond_to?` consults).
pub fn responds(v: &Value, name: &str) -> bool {
    if OBJECT_METHODS.contains(&name) {
        return true;
    }
    let (a, b) = type_methods(v);
    a.contains(&name) || b.contains(&name)
}

pub fn no_method<T>(recv: &Value, name: &str) -> R<T> {
    fail(
        NO_METHOD_ERROR,
        format!("undefined method '{}' for {}", name, recv.recv_desc()),
    )
}

/// Dispatches a method call through the allowlist.
pub fn call_method(it: &mut Interp, recv: &Value, name: &str, args: Vec<Value>, blk: Option<&BlockArg>) -> R<Value> {
    if !responds(recv, name) {
        return no_method(recv, name);
    }
    match recv {
        Value::Nil | Value::Bool(_) => object::call_nil_bool(it, recv, name, args, blk),
        Value::Int(i) => numeric::call_int(it, *i, recv, name, args, blk),
        Value::Float(f) => numeric::call_float(it, *f, recv, name, args, blk),
        Value::Str(s) => string::call_str(it, s, recv, name, args, blk),
        Value::Sym(s) => string::call_sym(it, s, recv, name, args, blk),
        Value::Array(_) => array::call(it, recv, name, args, blk),
        Value::Hash(_) => hash::call(it, recv, name, args, blk),
        Value::Range(_) => range::call(it, recv, name, args, blk),
        Value::Enumerator(_) => enumerator::call(it, recv, name, args, blk),
        Value::Class(c) => kernel::call_class(it, *c, recv, name, args, blk),
    }
}

/// Checks the argument count.
pub fn argc(args: &[Value], min: usize, max: usize) -> R<()> {
    if args.len() < min || args.len() > max {
        let expected = if min == max {
            min.to_string()
        } else if max == usize::MAX {
            format!("{}+", min)
        } else {
            format!("{}..{}", min, max)
        };
        return fail(
            ARGUMENT_ERROR,
            format!(
                "wrong number of arguments (given {}, expected {})",
                args.len(),
                expected
            ),
        );
    }
    Ok(())
}

/// Implicit Integer conversion for index-like arguments.
pub fn to_int(v: &Value) -> R<i64> {
    match v {
        Value::Int(i) => Ok(*i),
        Value::Float(f) => crate::format::float_to_int(*f),
        other => fail(
            TYPE_ERROR,
            format!("no implicit conversion of {} into Integer", other.type_desc()),
        ),
    }
}

/// Implicit String conversion.
pub fn to_str(v: &Value) -> R<Arc<str>> {
    match v {
        Value::Str(s) => Ok(s.clone()),
        other => fail(
            TYPE_ERROR,
            format!("no implicit conversion of {} into String", other.type_desc()),
        ),
    }
}

pub fn check_str_len(n: usize) -> R<()> {
    if n > MAX_STR_LEN {
        return fail(LIMIT_ERROR, "string too large");
    }
    Ok(())
}

pub fn check_array_len(n: usize) -> R<()> {
    if n > MAX_ARRAY_LEN {
        return fail(LIMIT_ERROR, "array too large");
    }
    Ok(())
}

/// Requires a block or returns an enumerator for `recv.name(*args)`.
macro_rules! block_or_enum {
    ($blk:expr, $recv:expr, $name:expr, $args:expr) => {
        match $blk {
            Some(b) => b,
            None => return Ok($crate::value::Value::enumerator($recv.clone(), $name, $args.clone())),
        }
    };
}
pub(crate) use block_or_enum;

/// All values an enumerable yields, as a vector (Hash pairs become `[k, v]` arrays).
pub fn iter_values(it: &mut Interp, v: &Value) -> R<Vec<Value>> {
    match v {
        Value::Array(a) => Ok(a.borrow().clone()),
        Value::Hash(h) => Ok(h
            .borrow()
            .iter()
            .map(|(k, v)| Value::array(vec![k.clone(), v.clone()]))
            .collect()),
        Value::Range(_) => range::to_vec(v),
        Value::Enumerator(_) => enumerator::to_vec(it, v),
        other => no_method(other, "each"),
    }
}

/// Packs the values given to a block into one value (several values become an Array).
pub fn pack(mut args: Vec<Value>) -> Value {
    if args.len() == 1 {
        args.pop().unwrap_or(Value::Nil)
    } else {
        Value::array(args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{render_with, Options, Vars};

    /// Calls every allowlisted method on every kind of receiver with assorted arguments,
    /// with and without a block. Errors are fine; panics are not.
    #[test]
    fn every_method_with_assorted_arguments_never_panics() {
        let receivers = [
            "nil",
            "true",
            "0",
            "-7",
            "2.5",
            "Float::NAN",
            "''",
            "'héllo wörld'",
            ":sym",
            "[]",
            "[3, 1, [2, nil], 'x']",
            "{}",
            "{a: 1, 'b' => [2]}",
            "(1..4)",
            "(1...)",
            "(1.5..2.5)",
            "[1, 2].each",
            "{a: 1}.each",
            "3.times",
            "Integer",
            "Math",
            "JSON",
            "File",
            "Dir",
            "Array",
            "Hash",
        ];
        let args = [
            "", "1", "-1", "2.5", "'a'", "nil", ":a", "[1, 2]", "{a: 1}", "(1..2)", "1, 2", "'a', 'b'", "1, 2, 3",
            "[[1, 2]]",
        ];
        let blocks = ["", " { |*| }", " { |a, b| a }", " { |x| nil }", "(&:to_s)"];
        let all: Vec<&str> = [
            OBJECT_METHODS,
            NIL_METHODS,
            BOOL_METHODS,
            INTEGER_METHODS,
            FLOAT_METHODS,
            STRING_METHODS,
            SYMBOL_METHODS,
            ENUMERABLE_METHODS,
            ARRAY_METHODS,
            HASH_METHODS,
            RANGE_METHODS,
            ENUMERATOR_METHODS,
            CLASS_METHODS,
            MATH_FUNCTIONS,
            JSON_FUNCTIONS,
            FILE_FUNCTIONS,
            DIR_FUNCTIONS,
            &["new"],
        ]
        .concat();
        let opts = Options {
            max_steps: Some(20_000),
            ..Default::default()
        };
        for r in receivers {
            for m in &all {
                for a in args {
                    for b in blocks {
                        if b.starts_with('(') && !a.is_empty() {
                            continue;
                        }
                        let call = if b.starts_with('(') {
                            format!("({}).{}{}", r, m, b)
                        } else {
                            format!("({}).{}({}){}", r, m, a, b.replace("|*|", "|x|"))
                        };
                        let src = format!("<%= x = {} %><%= x.inspect %>", call);
                        let t0 = std::time::Instant::now();
                        let res = std::panic::catch_unwind(|| {
                            let _ = render_with(&src, &Vars::new(), &opts);
                        });
                        assert!(res.is_ok(), "panic: {}", src);
                        assert!(t0.elapsed().as_secs() < 5, "too slow: {}", src);
                    }
                }
            }
        }
    }
}
