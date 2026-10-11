//! Runtime values, equality, hashing keys, comparison, `to_s` and `inspect`.

use std::sync::Arc;
use crate::error::{fail, Error, R, LIMIT_ERROR};
use crate::format::float_to_s;
use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;
use std::ops::Deref;
use std::rc::Rc;

/// Nesting depth past which recursive operations on data stop.
pub const MAX_DATA_DEPTH: usize = 1000;

/// A template value.
#[derive(Clone)]
pub enum Value {
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(Arc<str>),
    Sym(Arc<str>),
    Array(Rc<Arr>),
    Hash(Rc<HashCell>),
    Range(Rc<RangeVal>),
    Class(ClassId),
    Enumerator(Rc<EnumVal>),
}

/// Shared, mutable array storage. `frozen` only records that `freeze` was called.
pub struct Arr {
    v: RefCell<Vec<Value>>,
    pub frozen: Cell<bool>,
}

/// Shared, mutable hash storage.
pub struct HashCell {
    h: RefCell<RHash>,
    pub frozen: Cell<bool>,
}

impl Deref for Arr {
    type Target = RefCell<Vec<Value>>;
    fn deref(&self) -> &Self::Target {
        &self.v
    }
}

impl Deref for HashCell {
    type Target = RefCell<RHash>;
    fn deref(&self) -> &Self::Target {
        &self.h
    }
}

/// Moves the children of a value that is about to be dropped onto `stack` when this is
/// the last reference, so deeply nested data is freed iteratively instead of recursively.
fn take_children(v: Value, stack: &mut Vec<Value>) {
    match v {
        Value::Array(rc) => {
            if let Ok(arr) = Rc::try_unwrap(rc) {
                stack.append(&mut arr.v.borrow_mut());
            }
        }
        Value::Hash(rc) => {
            if let Ok(h) = Rc::try_unwrap(rc) {
                let mut inner = h.h.borrow_mut();
                for (k, v) in inner.entries.drain(..) {
                    stack.push(k);
                    stack.push(v);
                }
                stack.push(std::mem::replace(&mut inner.default, Value::Nil));
            }
        }
        Value::Enumerator(rc) => {
            if let Ok(mut e) = Rc::try_unwrap(rc) {
                stack.push(std::mem::replace(&mut e.recv, Value::Nil));
                stack.append(&mut e.args);
            }
        }
        _ => {}
    }
}

fn drain_iteratively(mut stack: Vec<Value>) {
    while let Some(v) = stack.pop() {
        take_children(v, &mut stack);
    }
}

impl Drop for Arr {
    fn drop(&mut self) {
        let items = std::mem::take(self.v.get_mut());
        if !items.is_empty() {
            drain_iteratively(items);
        }
    }
}

impl Drop for HashCell {
    fn drop(&mut self) {
        let h = self.h.get_mut();
        let mut stack = Vec::new();
        for (k, v) in h.entries.drain(..) {
            stack.push(k);
            stack.push(v);
        }
        stack.push(std::mem::replace(&mut h.default, Value::Nil));
        h.index.clear();
        drain_iteratively(stack);
    }
}

/// A range literal `a..b` / `a...b`. A `nil` endpoint means beginless / endless.
pub struct RangeVal {
    pub start: Value,
    pub end: Value,
    pub excl: bool,
}

/// An enumerator: a method call on `recv` that is waiting for a block.
pub struct EnumVal {
    pub recv: Value,
    pub meth: Arc<str>,
    pub args: Vec<Value>,
}

impl Drop for EnumVal {
    fn drop(&mut self) {
        let mut stack = std::mem::take(&mut self.args);
        stack.push(std::mem::replace(&mut self.recv, Value::Nil));
        drain_iteratively(stack);
    }
}

/// Built-in classes and modules visible to templates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ClassId {
    Object,
    Numeric,
    Integer,
    Float,
    String,
    Symbol,
    Array,
    Hash,
    Range,
    NilClass,
    TrueClass,
    FalseClass,
    Comparable,
    Enumerable,
    Enumerator,
    Class,
    Module,
    Math,
    Json,
    File,
    Dir,
}

impl ClassId {
    pub fn name(self) -> &'static str {
        match self {
            ClassId::Object => "Object",
            ClassId::Numeric => "Numeric",
            ClassId::Integer => "Integer",
            ClassId::Float => "Float",
            ClassId::String => "String",
            ClassId::Symbol => "Symbol",
            ClassId::Array => "Array",
            ClassId::Hash => "Hash",
            ClassId::Range => "Range",
            ClassId::NilClass => "NilClass",
            ClassId::TrueClass => "TrueClass",
            ClassId::FalseClass => "FalseClass",
            ClassId::Comparable => "Comparable",
            ClassId::Enumerable => "Enumerable",
            ClassId::Enumerator => "Enumerator",
            ClassId::Class => "Class",
            ClassId::Module => "Module",
            ClassId::Math => "Math",
            ClassId::Json => "JSON",
            ClassId::File => "File",
            ClassId::Dir => "Dir",
        }
    }

    /// Looks up a top-level constant by name.
    pub fn from_const(name: &str) -> Option<ClassId> {
        Some(match name {
            "Object" => ClassId::Object,
            "Numeric" => ClassId::Numeric,
            "Integer" => ClassId::Integer,
            "Float" => ClassId::Float,
            "String" => ClassId::String,
            "Symbol" => ClassId::Symbol,
            "Array" => ClassId::Array,
            "Hash" => ClassId::Hash,
            "Range" => ClassId::Range,
            "NilClass" => ClassId::NilClass,
            "TrueClass" => ClassId::TrueClass,
            "FalseClass" => ClassId::FalseClass,
            "Comparable" => ClassId::Comparable,
            "Enumerable" => ClassId::Enumerable,
            "Enumerator" => ClassId::Enumerator,
            "Math" => ClassId::Math,
            "JSON" => ClassId::Json,
            "File" => ClassId::File,
            "Dir" => ClassId::Dir,
            _ => return None,
        })
    }

    /// True for modules (as opposed to classes).
    pub fn is_module(self) -> bool {
        matches!(
            self,
            ClassId::Comparable | ClassId::Enumerable | ClassId::Math | ClassId::Json
        )
    }

    /// The class and the modules/superclasses it inherits from.
    pub fn ancestors(self) -> &'static [ClassId] {
        use ClassId::*;
        match self {
            Integer => &[Integer, Numeric, Comparable, Object],
            Float => &[Float, Numeric, Comparable, Object],
            Numeric => &[Numeric, Comparable, Object],
            String => &[String, Comparable, Object],
            Symbol => &[Symbol, Comparable, Object],
            Array => &[Array, Enumerable, Object],
            Hash => &[Hash, Enumerable, Object],
            Range => &[Range, Enumerable, Object],
            Enumerator => &[Enumerator, Enumerable, Object],
            NilClass => &[NilClass, Object],
            TrueClass => &[TrueClass, Object],
            FalseClass => &[FalseClass, Object],
            Class => &[Class, Module, Object],
            Module => &[Module, Object],
            File => &[File, Object],
            Dir => &[Dir, Object],
            Object => &[Object],
            Comparable => &[Comparable],
            Enumerable => &[Enumerable],
            Math => &[Math],
            Json => &[Json],
        }
    }
}

/// Hashable key with Ruby `eql?` semantics (so `1` and `1.0` differ).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum HKey {
    Nil,
    Bool(bool),
    Int(i64),
    Float(u64),
    Str(Arc<str>),
    Sym(Arc<str>),
    Arr(Vec<HKey>),
    Hash(Vec<(HKey, HKey)>),
    Range(Box<HKey>, Box<HKey>, bool),
    Class(ClassId),
    Ptr(usize),
}

/// Insertion-ordered hash table with a default value.
#[derive(Clone)]
pub struct RHash {
    entries: Vec<(Value, Value)>,
    index: HashMap<HKey, usize>,
    pub default: Value,
}

impl Default for RHash {
    fn default() -> Self {
        Self::new()
    }
}

impl RHash {
    pub fn new() -> RHash {
        RHash { entries: Vec::new(), index: HashMap::new(), default: Value::Nil }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, k: &Value) -> Option<Value> {
        self.index.get(&k.hkey()).map(|&i| self.entries[i].1.clone())
    }

    pub fn contains(&self, k: &Value) -> bool {
        self.index.contains_key(&k.hkey())
    }

    pub fn insert(&mut self, k: Value, v: Value) {
        let key = k.hkey();
        if let Some(&i) = self.index.get(&key) {
            self.entries[i].1 = v;
        } else {
            self.index.insert(key, self.entries.len());
            self.entries.push((k, v));
        }
    }

    pub fn remove(&mut self, k: &Value) -> Option<Value> {
        let i = self.index.remove(&k.hkey())?;
        let (_, v) = self.entries.remove(i);
        for (j, (key, _)) in self.entries.iter().enumerate().skip(i) {
            self.index.insert(key.hkey(), j);
        }
        Some(v)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.index.clear();
    }

    pub fn pairs(&self) -> Vec<(Value, Value)> {
        self.entries.clone()
    }

    pub fn iter(&self) -> impl Iterator<Item = &(Value, Value)> {
        self.entries.iter()
    }

    pub fn keys(&self) -> Vec<Value> {
        self.entries.iter().map(|(k, _)| k.clone()).collect()
    }

    pub fn values(&self) -> Vec<Value> {
        self.entries.iter().map(|(_, v)| v.clone()).collect()
    }

    pub fn first(&self) -> Option<(Value, Value)> {
        self.entries.first().cloned()
    }

    pub fn from_pairs(pairs: Vec<(Value, Value)>) -> RHash {
        let mut h = RHash::new();
        for (k, v) in pairs {
            h.insert(k, v);
        }
        h
    }
}

impl Value {
    pub fn str(s: &str) -> Value {
        Value::Str(Arc::from(s))
    }

    pub fn string(s: String) -> Value {
        Value::Str(Arc::from(s))
    }

    pub fn sym(s: &str) -> Value {
        Value::Sym(Arc::from(s))
    }

    pub fn array(v: Vec<Value>) -> Value {
        Value::Array(Rc::new(Arr { v: RefCell::new(v), frozen: Cell::new(false) }))
    }

    pub fn hash(h: RHash) -> Value {
        Value::Hash(Rc::new(HashCell { h: RefCell::new(h), frozen: Cell::new(false) }))
    }

    pub fn hash_from(pairs: Vec<(Value, Value)>) -> Value {
        Value::hash(RHash::from_pairs(pairs))
    }

    pub fn range(start: Value, end: Value, excl: bool) -> Value {
        Value::Range(Rc::new(RangeVal { start, end, excl }))
    }

    pub fn enumerator(recv: Value, meth: &str, args: Vec<Value>) -> Value {
        Value::Enumerator(Rc::new(EnumVal { recv, meth: Arc::from(meth), args }))
    }

    /// Parses JSON text into a value (objects become Hashes with String keys).
    pub fn from_json(text: &str) -> Result<Value, Error> {
        crate::json::parse(text, false).map_err(|f| crate::interp::flow_to_error(f, "(json)"))
    }

    pub fn truthy(&self) -> bool {
        !matches!(self, Value::Nil | Value::Bool(false))
    }

    pub fn is_nil(&self) -> bool {
        matches!(self, Value::Nil)
    }

    pub fn class_id(&self) -> ClassId {
        match self {
            Value::Nil => ClassId::NilClass,
            Value::Bool(true) => ClassId::TrueClass,
            Value::Bool(false) => ClassId::FalseClass,
            Value::Int(_) => ClassId::Integer,
            Value::Float(_) => ClassId::Float,
            Value::Str(_) => ClassId::String,
            Value::Sym(_) => ClassId::Symbol,
            Value::Array(_) => ClassId::Array,
            Value::Hash(_) => ClassId::Hash,
            Value::Range(_) => ClassId::Range,
            Value::Enumerator(_) => ClassId::Enumerator,
            Value::Class(c) => {
                if c.is_module() {
                    ClassId::Module
                } else {
                    ClassId::Class
                }
            }
        }
    }

    pub fn class_name(&self) -> &'static str {
        self.class_id().name()
    }

    /// Name used in "no implicit conversion of X" style messages.
    pub fn type_desc(&self) -> &'static str {
        match self {
            Value::Nil => "nil",
            Value::Bool(true) => "true",
            Value::Bool(false) => "false",
            _ => self.class_name(),
        }
    }

    /// Description used in NoMethodError messages.
    pub fn recv_desc(&self) -> String {
        match self {
            Value::Nil => "nil".into(),
            Value::Bool(b) => b.to_string(),
            Value::Class(c) => {
                if c.is_module() {
                    format!("module {}", c.name())
                } else {
                    format!("class {}", c.name())
                }
            }
            _ => format!("an instance of {}", self.class_name()),
        }
    }

    /// `frozen?` for values whose state does not depend on `freeze` having been called.
    pub fn is_frozen(&self) -> bool {
        match self {
            Value::Nil | Value::Bool(_) | Value::Int(_) | Value::Float(_) | Value::Sym(_) | Value::Range(_) => true,
            Value::Array(a) => a.frozen.get(),
            Value::Hash(h) => h.frozen.get(),
            _ => false,
        }
    }

    fn ptr_id(&self) -> usize {
        match self {
            Value::Array(a) => Rc::as_ptr(a) as *const u8 as usize,
            Value::Hash(h) => Rc::as_ptr(h) as *const u8 as usize,
            Value::Range(r) => Rc::as_ptr(r) as *const u8 as usize,
            Value::Enumerator(e) => Rc::as_ptr(e) as *const u8 as usize,
            Value::Str(s) => s.as_ptr() as usize,
            _ => 0,
        }
    }

    /// True when both values are the same object (`equal?`).
    pub fn same_object(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Nil, Value::Nil) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a.to_bits() == b.to_bits(),
            (Value::Sym(a), Value::Sym(b)) => a == b,
            (Value::Class(a), Value::Class(b)) => a == b,
            (Value::Str(a), Value::Str(b)) => Arc::ptr_eq(a, b),
            (Value::Array(a), Value::Array(b)) => Rc::ptr_eq(a, b),
            (Value::Hash(a), Value::Hash(b)) => Rc::ptr_eq(a, b),
            (Value::Range(a), Value::Range(b)) => Rc::ptr_eq(a, b),
            (Value::Enumerator(a), Value::Enumerator(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }

    /// Hash key with `eql?` semantics.
    pub fn hkey(&self) -> HKey {
        self.hkey_depth(0)
    }

    fn hkey_depth(&self, depth: usize) -> HKey {
        if depth > MAX_DATA_DEPTH || crate::stack::exhausted() {
            return HKey::Ptr(self.ptr_id());
        }
        match self {
            Value::Nil => HKey::Nil,
            Value::Bool(b) => HKey::Bool(*b),
            Value::Int(i) => HKey::Int(*i),
            Value::Float(f) => HKey::Float(if *f == 0.0 { 0 } else { f.to_bits() }),
            Value::Str(s) => HKey::Str(s.clone()),
            Value::Sym(s) => HKey::Sym(s.clone()),
            Value::Array(a) => match a.try_borrow() {
                Ok(items) => HKey::Arr(items.iter().map(|v| v.hkey_depth(depth + 1)).collect()),
                Err(_) => HKey::Ptr(self.ptr_id()),
            },
            Value::Hash(h) => match h.try_borrow() {
                Ok(h) => HKey::Hash(
                    h.iter()
                        .map(|(k, v)| (k.hkey_depth(depth + 1), v.hkey_depth(depth + 1)))
                        .collect(),
                ),
                Err(_) => HKey::Ptr(self.ptr_id()),
            },
            Value::Range(r) => HKey::Range(
                Box::new(r.start.hkey_depth(depth + 1)),
                Box::new(r.end.hkey_depth(depth + 1)),
                r.excl,
            ),
            Value::Class(c) => HKey::Class(*c),
            Value::Enumerator(_) => HKey::Ptr(self.ptr_id()),
        }
    }

    /// Ruby `eql?`.
    pub fn eql(&self, other: &Value) -> bool {
        if let (Value::Float(a), Value::Float(b)) = (self, other) {
            return a == b;
        }
        self.hkey() == other.hkey()
    }

    /// Ruby `==`.
    pub fn equals(&self, other: &Value) -> bool {
        values_eq(self, other, 0)
    }

    /// Ruby `<=>` for the built-in comparable types; `None` when not comparable.
    pub fn compare(&self, other: &Value) -> Option<Ordering> {
        compare_depth(self, other, 0)
    }

    /// `to_s` conversion.
    pub fn to_s(&self) -> R<String> {
        match self {
            Value::Nil => Ok(String::new()),
            Value::Str(s) => Ok(s.to_string()),
            Value::Sym(s) => Ok(s.to_string()),
            Value::Range(r) => {
                let mut s = r.start.to_s()?;
                s.push_str(if r.excl { "..." } else { ".." });
                s.push_str(&r.end.to_s()?);
                Ok(s)
            }
            _ => self.inspect(),
        }
    }

    /// `inspect` conversion.
    pub fn inspect(&self) -> R<String> {
        let mut out = String::new();
        let mut seen = Vec::new();
        inspect_into(self, &mut out, &mut seen)?;
        Ok(out)
    }
}

fn values_eq(a: &Value, b: &Value, depth: usize) -> bool {
    if depth > MAX_DATA_DEPTH || crate::stack::exhausted() {
        return a.same_object(b);
    }
    match (a, b) {
        (Value::Nil, Value::Nil) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Float(x), Value::Float(y)) => x == y,
        (Value::Int(x), Value::Float(y)) | (Value::Float(y), Value::Int(x)) => {
            cmp_int_float(*x, *y) == Some(Ordering::Equal)
        }
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Sym(x), Value::Sym(y)) => x == y,
        (Value::Class(x), Value::Class(y)) => x == y,
        (Value::Array(x), Value::Array(y)) => {
            if Rc::ptr_eq(x, y) {
                return true;
            }
            let (Ok(xs), Ok(ys)) = (x.try_borrow(), y.try_borrow()) else {
                return false;
            };
            xs.len() == ys.len() && xs.iter().zip(ys.iter()).all(|(p, q)| values_eq(p, q, depth + 1))
        }
        (Value::Hash(x), Value::Hash(y)) => {
            if Rc::ptr_eq(x, y) {
                return true;
            }
            let (Ok(xs), Ok(ys)) = (x.try_borrow(), y.try_borrow()) else {
                return false;
            };
            xs.len() == ys.len()
                && xs.iter().all(|(k, v)| match ys.get(k) {
                    Some(w) => values_eq(v, &w, depth + 1),
                    None => false,
                })
        }
        (Value::Range(x), Value::Range(y)) => {
            x.excl == y.excl
                && values_eq(&x.start, &y.start, depth + 1)
                && values_eq(&x.end, &y.end, depth + 1)
        }
        (Value::Enumerator(x), Value::Enumerator(y)) => Rc::ptr_eq(x, y),
        _ => false,
    }
}

/// Exact comparison of an integer with a float.
pub fn cmp_int_float(i: i64, f: f64) -> Option<Ordering> {
    if f.is_nan() {
        return None;
    }
    if f >= crate::format::TWO_POW_63 {
        return Some(Ordering::Less);
    }
    if f < -crate::format::TWO_POW_63 {
        return Some(Ordering::Greater);
    }
    let t = f.trunc();
    let ti = t as i64;
    match i.cmp(&ti) {
        Ordering::Equal => {
            let frac = f - t;
            if frac > 0.0 {
                Some(Ordering::Less)
            } else if frac < 0.0 {
                Some(Ordering::Greater)
            } else {
                Some(Ordering::Equal)
            }
        }
        o => Some(o),
    }
}

fn compare_depth(a: &Value, b: &Value, depth: usize) -> Option<Ordering> {
    if depth > MAX_DATA_DEPTH || crate::stack::exhausted() {
        return None;
    }
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => Some(x.cmp(y)),
        (Value::Float(x), Value::Float(y)) => x.partial_cmp(y),
        (Value::Int(x), Value::Float(y)) => cmp_int_float(*x, *y),
        (Value::Float(x), Value::Int(y)) => cmp_int_float(*y, *x).map(Ordering::reverse),
        (Value::Str(x), Value::Str(y)) => Some(x.as_bytes().cmp(y.as_bytes())),
        (Value::Sym(x), Value::Sym(y)) => Some(x.as_bytes().cmp(y.as_bytes())),
        (Value::Array(x), Value::Array(y)) => {
            let xs = x.try_borrow().ok()?.clone();
            let ys = y.try_borrow().ok()?.clone();
            for (p, q) in xs.iter().zip(ys.iter()) {
                match compare_depth(p, q, depth + 1)? {
                    Ordering::Equal => {}
                    o => return Some(o),
                }
            }
            Some(xs.len().cmp(&ys.len()))
        }
        _ => {
            if values_eq(a, b, depth) {
                Some(Ordering::Equal)
            } else {
                None
            }
        }
    }
}

/// True when `name` can be written as a bare symbol (`:name`).
pub fn symbol_is_simple(name: &str) -> bool {
    const OPS: &[&str] = &[
        "+", "-", "*", "/", "%", "**", "==", "!=", "<", ">", "<=", ">=", "<=>", "!", "[]", "[]=", "<<",
        ">>", "&", "|", "^", "~", "+@", "-@", "===", "=~", "!~",
    ];
    if OPS.contains(&name) {
        return true;
    }
    let var = name.strip_prefix("@@").or_else(|| name.strip_prefix('@')).or_else(|| name.strip_prefix('$'));
    if let Some(v) = var {
        return symbol_is_label(v) && !v.ends_with(['?', '!']) && !v.starts_with(|c: char| c.is_ascii_digit());
    }
    symbol_is_label(name) || {
        let base = name.strip_suffix('=').unwrap_or("");
        !base.is_empty() && !base.ends_with(['?', '!']) && symbol_is_label(base)
    }
}

/// True when a symbol key can be written as a label in hash inspect (`name: v`).
pub fn symbol_is_label(name: &str) -> bool {
    let mut chars = name.chars().peekable();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' || !c.is_ascii() => {}
        _ => return false,
    }
    while let Some(c) = chars.next() {
        if c.is_ascii_alphanumeric() || c == '_' || !c.is_ascii() {
            continue;
        }
        if (c == '?' || c == '!') && chars.peek().is_none() {
            return true;
        }
        return false;
    }
    true
}

/// Appends a Ruby-style quoted, escaped string.
pub fn inspect_str(s: &str, out: &mut String) {
    out.push('"');
    let chars: Vec<char> = s.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\x1b' => out.push_str("\\e"),
            '\x07' => out.push_str("\\a"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            '\x0b' => out.push_str("\\v"),
            '#' => {
                if matches!(chars.get(i + 1), Some('{') | Some('$') | Some('@')) {
                    out.push_str("\\#");
                } else {
                    out.push('#');
                }
            }
            c if is_unprintable(c) => {
                let cp = c as u32;
                if cp > 0xFFFF {
                    out.push_str(&format!("\\u{{{:X}}}", cp));
                } else {
                    out.push_str(&format!("\\u{:04X}", cp));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn is_unprintable(c: char) -> bool {
    let cp = c as u32;
    cp < 0x20
        || (0x7F..=0x9F).contains(&cp)
        || cp == 0x2028
        || cp == 0x2029
        || (0xFDD0..=0xFDEF).contains(&cp)
        || (cp & 0xFFFE) == 0xFFFE
}

/// Appends a symbol in `:name` or `:"quoted"` form.
pub fn inspect_sym(name: &str, out: &mut String) {
    out.push(':');
    if symbol_is_simple(name) {
        out.push_str(name);
    } else {
        inspect_str(name, out);
    }
}

fn inspect_into(v: &Value, out: &mut String, seen: &mut Vec<usize>) -> R<()> {
    if seen.len() > MAX_DATA_DEPTH || crate::stack::exhausted() {
        return fail(LIMIT_ERROR, "nesting too deep to inspect");
    }
    match v {
        Value::Nil => out.push_str("nil"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(i) => out.push_str(&i.to_string()),
        Value::Float(f) => out.push_str(&float_to_s(*f)),
        Value::Str(s) => inspect_str(s, out),
        Value::Sym(s) => inspect_sym(s, out),
        Value::Class(c) => out.push_str(c.name()),
        Value::Array(a) => {
            let id = v.ptr_id();
            if seen.contains(&id) {
                out.push_str("[...]");
                return Ok(());
            }
            let items = a.borrow().clone();
            seen.push(id);
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                inspect_into(item, out, seen)?;
            }
            out.push(']');
            seen.pop();
        }
        Value::Hash(h) => {
            let id = v.ptr_id();
            if seen.contains(&id) {
                out.push_str("{...}");
                return Ok(());
            }
            let pairs = h.borrow().pairs();
            if pairs.is_empty() {
                out.push_str("{}");
                return Ok(());
            }
            seen.push(id);
            out.push('{');
            for (i, (k, val)) in pairs.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                match k {
                    Value::Sym(s) if symbol_is_label(s) => {
                        out.push_str(s);
                        out.push_str(": ");
                    }
                    Value::Sym(s) => {
                        inspect_str(s, out);
                        out.push_str(": ");
                    }
                    _ => {
                        inspect_into(k, out, seen)?;
                        out.push_str(" => ");
                    }
                }
                inspect_into(val, out, seen)?;
            }
            out.push('}');
            seen.pop();
        }
        Value::Range(r) => {
            if !(r.start.is_nil() && !r.end.is_nil()) {
                inspect_into(&r.start, out, seen)?;
            }
            out.push_str(if r.excl { "..." } else { ".." });
            if !r.end.is_nil() || r.start.is_nil() {
                inspect_into(&r.end, out, seen)?;
            }
        }
        Value::Enumerator(e) if &*e.meth == "step" && matches!(e.recv, Value::Int(_) | Value::Float(_) | Value::Range(_)) => {
            // an ArithmeticSequence: (1.step(10, 3)) / ((1..10).step(2))
            out.push('(');
            let is_range = matches!(e.recv, Value::Range(_));
            if is_range {
                out.push('(');
            }
            inspect_into(&e.recv, out, seen)?;
            if is_range {
                out.push(')');
            }
            out.push_str(".step");
            if !e.args.is_empty() {
                out.push('(');
                for (i, a) in e.args.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    inspect_into(a, out, seen)?;
                }
                out.push(')');
            }
            out.push(')');
        }
        Value::Enumerator(e) => {
            out.push_str("#<Enumerator: ");
            seen.push(v.ptr_id());
            inspect_into(&e.recv, out, seen)?;
            seen.pop();
            out.push(':');
            out.push_str(&e.meth);
            if !e.args.is_empty() {
                out.push('(');
                for (i, a) in e.args.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    inspect_into(a, out, seen)?;
                }
                out.push(')');
            }
            out.push('>');
        }
    }
    Ok(())
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.inspect() {
            Ok(s) => f.write_str(&s),
            Err(_) => f.write_str("#<deeply nested value>"),
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Value) -> bool {
        self.equals(other)
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Value {
        Value::Int(v)
    }
}

impl From<i32> for Value {
    fn from(v: i32) -> Value {
        Value::Int(v as i64)
    }
}

impl From<u32> for Value {
    fn from(v: u32) -> Value {
        Value::Int(v as i64)
    }
}

impl From<f64> for Value {
    fn from(v: f64) -> Value {
        Value::Float(v)
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Value {
        Value::Bool(v)
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Value {
        Value::str(v)
    }
}

impl From<String> for Value {
    fn from(v: String) -> Value {
        Value::string(v)
    }
}

impl<T: Into<Value>> From<Vec<T>> for Value {
    fn from(v: Vec<T>) -> Value {
        Value::array(v.into_iter().map(Into::into).collect())
    }
}

impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(v: Option<T>) -> Value {
        match v {
            Some(x) => x.into(),
            None => Value::Nil,
        }
    }
}

impl From<()> for Value {
    fn from(_: ()) -> Value {
        Value::Nil
    }
}

impl<K: Into<String>, V: Into<Value>> From<HashMap<K, V>> for Value {
    fn from(map: HashMap<K, V>) -> Value {
        let mut h = RHash::new();
        for (k, v) in map {
            h.insert(Value::string(k.into()), v.into());
        }
        Value::hash(h)
    }
}

impl<K: Into<String>, V: Into<Value>> From<std::collections::BTreeMap<K, V>> for Value {
    fn from(map: std::collections::BTreeMap<K, V>) -> Value {
        let mut h = RHash::new();
        for (k, v) in map {
            h.insert(Value::string(k.into()), v.into());
        }
        Value::hash(h)
    }
}
