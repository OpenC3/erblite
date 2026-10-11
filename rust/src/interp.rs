// Copyright 2026 OpenC3, Inc.
// All Rights Reserved.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
// See LICENSE.md for more details.

//! Tree-walking interpreter.

use crate::ast::{Arg, BlockNode, Kind, Node, Param, Target, VarRef};
use crate::builtins;
use crate::error::{fail, rt_err, Error, Flow, ARGUMENT_ERROR, LIMIT_ERROR, NAME_ERROR, R, RANGE_ERROR};
use crate::value::{ClassId, RHash, Value};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

/// Deepest allowed expression evaluation nesting.
const MAX_EVAL_DEPTH: u32 = 3000;
/// Deepest allowed block nesting at run time.
const MAX_FRAMES: usize = 400;
/// Deepest allowed `render` nesting.
pub const MAX_RENDER_DEPTH: u32 = 32;

/// State shared by a render call and all partials it renders.
pub struct Ctx {
    /// Canonical sandbox root, or `None` when file access is disabled.
    pub base_dir: Option<PathBuf>,
    pub max_steps: Option<u64>,
    pub steps: Cell<u64>,
    /// Strings that `freeze` was called on (strings are immutable values, so freezing only
    /// affects what `frozen?` reports).
    pub frozen_strs: RefCell<Vec<std::sync::Weak<str>>>,
}

impl Ctx {
    pub fn new(base_dir: Option<PathBuf>, max_steps: Option<u64>) -> Ctx {
        Ctx {
            base_dir,
            max_steps,
            steps: Cell::new(0),
            frozen_strs: RefCell::new(Vec::new()),
        }
    }

    /// Returns a frozen copy of a string.
    pub fn freeze_str(&self, s: &str) -> Arc<str> {
        let rc: Arc<str> = Arc::from(s);
        let mut reg = self.frozen_strs.borrow_mut();
        if reg.len() >= 64 && reg.len().is_power_of_two() {
            reg.retain(|w| w.strong_count() > 0);
        }
        reg.push(Arc::downgrade(&rc));
        rc
    }

    pub fn is_frozen_str(&self, s: &Arc<str>) -> bool {
        self.frozen_strs
            .borrow()
            .iter()
            .any(|w| w.upgrade().is_some_and(|x| Arc::ptr_eq(&x, s)))
    }
}

/// A block passed to a built-in method.
pub enum BlockArg<'b> {
    /// A `{ }` / `do end` block from the template.
    Ast(&'b BlockNode),
    /// `&:sym`: calls the method on the first yielded value.
    Sym(Arc<str>),
    /// A block implemented by the interpreter itself (used by enumerators).
    Native(&'b dyn Fn(&mut Interp, Vec<Value>) -> R<Value>),
}

pub struct Interp {
    pub ctx: Rc<Ctx>,
    pub out: String,
    frames: Vec<HashMap<Arc<str>, Value>>,
    /// Values of text and `<%= %>` statements: a copy of the output buffer at that moment.
    /// They are created as empty marker strings that remember the buffer length and are
    /// turned into the copy only when used (the buffer only grows).
    buffer_marks: HashMap<usize, (Arc<str>, usize)>,
    buffer_marks_limit: usize,
    pub filename: Arc<str>,
    depth: u32,
    pub render_depth: u32,
}

enum Setter {
    Var(VarRef),
    Index(Value, Vec<Value>),
}

/// Converts an internal error into the public error type.
pub fn flow_to_error(f: Flow, filename: &str) -> Error {
    match f {
        Flow::Err(e) => Error {
            kind: e.kind.to_string(),
            message: e.message,
            filename: e.filename.unwrap_or_else(|| filename.to_string()),
            line: e.line.unwrap_or(1) as usize,
        },
        Flow::Break(_) | Flow::Next(_) => Error {
            kind: "SyntaxError".into(),
            message: "break or next used outside of a loop or block".into(),
            filename: filename.to_string(),
            line: 1,
        },
    }
}

impl Interp {
    pub fn new(ctx: Rc<Ctx>, filename: &str, vars: Vec<(Arc<str>, Value)>, render_depth: u32) -> Interp {
        let mut top = HashMap::new();
        for (k, v) in vars {
            top.insert(k, v);
        }
        Interp {
            ctx,
            out: String::new(),
            frames: vec![top],
            buffer_marks: HashMap::new(),
            buffer_marks_limit: 64,
            filename: Arc::from(filename),
            depth: 0,
            render_depth,
        }
    }

    /// Runs a parsed program and returns the rendered output.
    pub fn run(&mut self, program: &[Node]) -> R<String> {
        for node in program {
            self.eval(node)?;
        }
        Ok(std::mem::take(&mut self.out))
    }

    pub fn tick(&mut self) -> R<()> {
        let s = self.ctx.steps.get() + 1;
        self.ctx.steps.set(s);
        if let Some(max) = self.ctx.max_steps {
            if s > max {
                return fail(LIMIT_ERROR, format!("execution step limit of {} exceeded", max));
            }
        }
        Ok(())
    }

    pub fn eval(&mut self, node: &Node) -> R<Value> {
        self.tick()?;
        self.depth += 1;
        let r = if self.depth > MAX_EVAL_DEPTH || crate::stack::exhausted() {
            fail(LIMIT_ERROR, "expression nesting too deep")
        } else {
            self.eval_kind(node)
        };
        self.depth -= 1;
        match r {
            Err(Flow::Err(mut e)) if e.line.is_none() => {
                e.line = Some(node.line);
                if e.filename.is_none() {
                    e.filename = Some(self.filename.to_string());
                }
                Err(Flow::Err(e))
            }
            other => other,
        }
    }

    pub fn eval_body(&mut self, nodes: &[Node]) -> R<Value> {
        let mut last = Value::Nil;
        for n in nodes {
            last = self.eval(n)?;
        }
        Ok(last)
    }

    fn read_var(&self, v: &VarRef) -> Value {
        let val = self
            .frames
            .get(v.depth)
            .and_then(|f| f.get(&v.name))
            .cloned()
            .unwrap_or(Value::Nil);
        self.materialize(val)
    }

    fn write_var(&mut self, v: &VarRef, val: Value) {
        if let Some(f) = self.frames.get_mut(v.depth) {
            f.insert(v.name.clone(), val);
        }
    }

    fn buffer_value(&mut self) -> Value {
        if self.buffer_marks.len() >= self.buffer_marks_limit {
            // forget marks no value refers to any more
            self.buffer_marks.retain(|_, (a, _)| Arc::strong_count(a) > 1);
            self.buffer_marks_limit = (self.buffer_marks.len() * 2).max(64);
        }
        let mark: Arc<str> = Arc::from("");
        let key = Arc::as_ptr(&mark) as *const u8 as usize;
        self.buffer_marks.insert(key, (mark.clone(), self.out.len()));
        Value::Str(mark)
    }

    /// Replaces a text-statement marker with the copy of the buffer it stands for.
    pub fn materialize(&self, v: Value) -> Value {
        if let Value::Str(s) = &v {
            if s.is_empty() && !self.buffer_marks.is_empty() {
                if let Some((_, len)) = self.buffer_marks.get(&(Arc::as_ptr(s) as *const u8 as usize)) {
                    return Value::str(self.out.get(..*len).unwrap_or(""));
                }
            }
        }
        v
    }

    fn eval_kind(&mut self, node: &Node) -> R<Value> {
        match &node.kind {
            Kind::Nil => Ok(Value::Nil),
            Kind::True => Ok(Value::Bool(true)),
            Kind::False => Ok(Value::Bool(false)),
            Kind::Int(i) => Ok(Value::Int(*i)),
            Kind::IntOverflow => fail(RANGE_ERROR, "integer literal out of 64-bit range"),
            Kind::Float(f) => Ok(Value::Float(*f)),
            Kind::Str(s) => Ok(Value::Str(s.clone())),
            Kind::Sym(s) => Ok(Value::Sym(s.clone())),
            Kind::DStr(parts) => Ok(Value::string(self.interpolate(parts)?)),
            Kind::DSym(parts) => Ok(Value::sym(&self.interpolate(parts)?)),
            Kind::Array(items) => {
                let mut v = Vec::with_capacity(items.len());
                for i in items {
                    let x = self.eval(i)?;
                    v.push(self.materialize(x));
                }
                Ok(Value::array(v))
            }
            Kind::Hash(pairs) => {
                let mut h = RHash::new();
                for (k, v) in pairs {
                    let k = self.eval(k)?;
                    let v = self.eval(v)?;
                    h.insert(self.materialize(k), self.materialize(v));
                }
                Ok(Value::hash(h))
            }
            Kind::Range(a, b, excl) => {
                let a = match a {
                    Some(n) => self.eval(n)?,
                    None => Value::Nil,
                };
                let b = match b {
                    Some(n) => self.eval(n)?,
                    None => Value::Nil,
                };
                make_range(a, b, *excl)
            }
            Kind::Text(s) => {
                self.out.push_str(s);
                builtins::check_str_len(self.out.len())?;
                Ok(self.buffer_value())
            }
            Kind::Output(inner) => {
                let v = self.eval(inner)?;
                let v = self.materialize(v);
                let s = v.to_s()?;
                self.out.push_str(&s);
                builtins::check_str_len(self.out.len())?;
                Ok(self.buffer_value())
            }
            Kind::Var(v) => Ok(self.read_var(v)),
            Kind::Assign(v, rhs) => {
                let val = self.eval(rhs)?;
                self.write_var(v, val.clone());
                Ok(val)
            }
            Kind::IdentCall {
                name,
                args,
                has_args,
                block,
            } => {
                let (argv, bp) = self.eval_args(args)?;
                let bare = !*has_args && block.is_none() && bp.is_none();
                match (block, bp) {
                    (Some(b), _) => {
                        let ba = BlockArg::Ast(b);
                        catch_break(builtins::kernel::call_function(self, name, argv, Some(&ba), false))
                    }
                    (None, Some(sym)) => {
                        let ba = BlockArg::Sym(sym);
                        builtins::kernel::call_function(self, name, argv, Some(&ba), false)
                    }
                    (None, None) => builtins::kernel::call_function(self, name, argv, None, bare),
                }
            }
            Kind::Call {
                recv,
                name,
                args,
                block,
                safe,
            } => {
                if &**name == "<<" && args.len() == 1 && block.is_none() && !*safe {
                    return Ok(self.shovel(node)?.0);
                }
                let r = self.eval(recv)?;
                if *safe && r.is_nil() {
                    return Ok(Value::Nil);
                }
                let (argv, bp) = self.eval_args(args)?;
                self.call_with_block(&r, name, argv, block.as_deref(), bp)
            }
            Kind::Const(name) => match ClassId::from_const(name) {
                Some(c) => Ok(Value::Class(c)),
                None => fail(NAME_ERROR, format!("uninitialized constant {}", name)),
            },
            Kind::ScopedConst(base, name) => {
                let b = self.eval(base)?;
                builtins::kernel::scoped_const(&b, name)
            }
            Kind::IndexAssign(recv, args, val) => {
                let r = self.eval(recv)?;
                let (mut argv, _) = self.eval_args(args)?;
                let v = self.eval(val)?;
                argv.push(v.clone());
                self.call_method(&r, "[]=", argv, None)?;
                Ok(v)
            }
            Kind::AttrAssign(recv, name, val) => {
                let r = self.eval(recv)?;
                let v = self.eval(val)?;
                let setter = format!("{}=", name);
                self.call_method(&r, &setter, vec![v.clone()], None)?;
                Ok(v)
            }
            Kind::OpAssign(target, op, rhs) => self.op_assign(target, op, rhs),
            Kind::MultiAssign(targets, values) => {
                let rhs = if values.len() == 1 {
                    self.eval(&values[0])?
                } else {
                    let mut v = Vec::new();
                    for n in values {
                        v.push(self.eval(n)?);
                    }
                    Value::array(v)
                };
                let items = match &rhs {
                    Value::Array(a) => a.borrow().clone(),
                    other => vec![other.clone()],
                };
                for (i, t) in targets.iter().enumerate() {
                    self.write_var(t, items.get(i).cloned().unwrap_or(Value::Nil));
                }
                Ok(rhs)
            }
            Kind::And(a, b) => {
                let l = self.eval(a)?;
                if l.truthy() {
                    self.eval(b)
                } else {
                    Ok(l)
                }
            }
            Kind::Or(a, b) => {
                let l = self.eval(a)?;
                if l.truthy() {
                    Ok(l)
                } else {
                    self.eval(b)
                }
            }
            Kind::Not(a) => Ok(Value::Bool(!self.eval(a)?.truthy())),
            Kind::If(cond, then, els) => {
                if self.eval(cond)?.truthy() {
                    self.eval_body(then)
                } else {
                    self.eval_body(els)
                }
            }
            Kind::While { cond, body, until } => {
                loop {
                    self.tick()?;
                    if self.eval(cond)?.truthy() == *until {
                        break;
                    }
                    match self.eval_body(body) {
                        Ok(_) | Err(Flow::Next(_)) => {}
                        Err(Flow::Break(v)) => return Ok(v),
                        Err(e) => return Err(e),
                    }
                }
                Ok(Value::Nil)
            }
            Kind::For { vars, iter, body } => self.eval_for(vars, iter, body),
            Kind::Case { subject, whens, els } => {
                let subj = match subject {
                    Some(s) => Some(self.eval(s)?),
                    None => None,
                };
                for (vals, body) in whens {
                    for v in vals {
                        let pat = self.eval(v)?;
                        let hit = match &subj {
                            Some(s) => self.call_method(&pat, "===", vec![s.clone()], None)?.truthy(),
                            None => pat.truthy(),
                        };
                        if hit {
                            return self.eval_body(body);
                        }
                    }
                }
                match els {
                    Some(b) => self.eval_body(b),
                    None => Ok(Value::Nil),
                }
            }
            Kind::Break(v) => {
                let val = match v {
                    Some(n) => self.eval(n)?,
                    None => Value::Nil,
                };
                Err(Flow::Break(val))
            }
            Kind::Next(v) => {
                let val = match v {
                    Some(n) => self.eval(n)?,
                    None => Value::Nil,
                };
                Err(Flow::Next(val))
            }
            Kind::Seq(nodes) => self.eval_body(nodes),
        }
    }

    fn interpolate(&mut self, parts: &[Node]) -> R<String> {
        let mut s = String::new();
        for p in parts {
            let v = self.eval(p)?;
            s.push_str(&self.materialize(v).to_s()?);
            builtins::check_str_len(s.len())?;
        }
        Ok(s)
    }

    /// Evaluates call arguments. `key: v` pairs become one trailing Hash argument.
    pub fn eval_args(&mut self, args: &[Arg]) -> R<(Vec<Value>, Option<Arc<str>>)> {
        let mut out = Vec::with_capacity(args.len());
        let mut pairs: Option<RHash> = None;
        let mut bp = None;
        for a in args {
            match a {
                Arg::Pos(n) => out.push(self.eval(n)?),
                Arg::Pair(k, v) => {
                    let k = self.eval(k)?;
                    let v = self.eval(v)?;
                    pairs.get_or_insert_with(RHash::new).insert(k, v);
                }
                Arg::BlockPass(s) => bp = Some(s.clone()),
            }
        }
        if let Some(h) = pairs {
            out.push(Value::hash(h));
        }
        Ok((out, bp))
    }

    fn call_with_block(
        &mut self,
        recv: &Value,
        name: &str,
        argv: Vec<Value>,
        block: Option<&BlockNode>,
        bp: Option<Arc<str>>,
    ) -> R<Value> {
        match (block, bp) {
            (Some(b), _) => {
                let ba = BlockArg::Ast(b);
                catch_break(self.call_method(recv, name, argv, Some(&ba)))
            }
            (None, Some(sym)) => {
                let ba = BlockArg::Sym(sym);
                self.call_method(recv, name, argv, Some(&ba))
            }
            (None, None) => self.call_method(recv, name, argv, None),
        }
    }

    /// Calls a built-in method on a value (allowlist dispatch).
    pub fn call_method(&mut self, recv: &Value, name: &str, args: Vec<Value>, blk: Option<&BlockArg>) -> R<Value> {
        let recv = self.materialize(recv.clone());
        let args = args.into_iter().map(|a| self.materialize(a)).collect();
        builtins::call_method(self, &recv, name, args, blk)
    }

    /// Invokes a block with the given yielded values.
    pub fn call_block(&mut self, blk: &BlockArg, args: Vec<Value>) -> R<Value> {
        self.tick()?;
        match blk {
            BlockArg::Ast(b) => {
                if self.frames.len() >= MAX_FRAMES || crate::stack::exhausted() {
                    return fail(LIMIT_ERROR, "block nesting too deep");
                }
                let mut frame = HashMap::new();
                bind_params(&b.params, args, b.splat, &mut frame);
                self.frames.push(frame);
                let r = self.eval_body(&b.body);
                self.frames.pop();
                match r {
                    Err(Flow::Next(v)) => Ok(v),
                    other => other,
                }
            }
            BlockArg::Sym(name) => {
                let mut args = args.into_iter();
                let Some(recv) = args.next() else {
                    return fail(ARGUMENT_ERROR, "no receiver given");
                };
                self.call_method(&recv, name, args.collect(), None)
            }
            BlockArg::Native(f) => f(self, args),
        }
    }

    /// Calls a block with one value.
    pub fn yield1(&mut self, blk: &BlockArg, v: Value) -> R<Value> {
        self.call_block(blk, vec![v])
    }

    /// Calls a block with one value; for results that are stored in a container.
    pub fn yield_keep(&mut self, blk: &BlockArg, v: Value) -> R<Value> {
        let r = self.call_block(blk, vec![v])?;
        Ok(self.materialize(r))
    }

    /// Evaluates `recv << arg`. A String receiver that can be assigned to is replaced by
    /// the concatenation (strings are values in ERBLite).
    fn shovel(&mut self, node: &Node) -> R<(Value, Option<Setter>)> {
        let Kind::Call { recv, args, .. } = &node.kind else {
            return Ok((self.eval(node)?, None));
        };
        let Some(Arg::Pos(arg)) = args.first() else {
            return Ok((self.eval(node)?, None));
        };
        self.tick()?;
        let (cur, setter) = match &recv.kind {
            Kind::Var(v) => (self.read_var(v), Some(Setter::Var(v.clone()))),
            Kind::Call {
                recv: r2,
                name,
                args: a2,
                block: None,
                safe: false,
            } if &**name == "[]" => {
                let rv = self.eval(r2)?;
                let (av, _) = self.eval_args(a2)?;
                let cur = self.call_method(&rv, "[]", av.clone(), None)?;
                (cur, Some(Setter::Index(rv, av)))
            }
            Kind::Call {
                name,
                args: a2,
                block: None,
                safe: false,
                ..
            } if &**name == "<<" && a2.len() == 1 => self.shovel(recv)?,
            _ => (self.eval(recv)?, None),
        };
        let argv = self.eval(arg)?;
        if let Value::Str(s) = &cur {
            let add = builtins::string::shovel_arg(&argv)?;
            builtins::check_str_len(s.len() + add.len())?;
            let mut n = String::with_capacity(s.len() + add.len());
            n.push_str(s);
            n.push_str(&add);
            let new = Value::string(n);
            if let Some(set) = &setter {
                self.apply_setter(set, new.clone())?;
            }
            return Ok((new, setter));
        }
        let r = self.call_method(&cur, "<<", vec![argv], None)?;
        Ok((r, None))
    }

    fn apply_setter(&mut self, set: &Setter, v: Value) -> R<()> {
        match set {
            Setter::Var(var) => self.write_var(var, v),
            Setter::Index(rv, av) => {
                let mut a = av.clone();
                a.push(v);
                self.call_method(rv, "[]=", a, None)?;
            }
        }
        Ok(())
    }

    fn op_assign(&mut self, target: &Target, op: &str, rhs: &Node) -> R<Value> {
        let (cur, setter) = match target {
            Target::Var(v) => (self.read_var(v), Setter::Var(v.clone())),
            Target::Index(recv, args) => {
                let rv = self.eval(recv)?;
                let (av, _) = self.eval_args(args)?;
                let cur = self.call_method(&rv, "[]", av.clone(), None)?;
                (cur, Setter::Index(rv, av))
            }
            Target::Attr(recv, name) => {
                let rv = self.eval(recv)?;
                let cur = self.call_method(&rv, name, vec![], None)?;
                let val = match op {
                    "||" if cur.truthy() => return Ok(cur),
                    "&&" if !cur.truthy() => return Ok(cur),
                    "||" | "&&" => self.eval(rhs)?,
                    _ => {
                        let r = self.eval(rhs)?;
                        self.call_method(&cur, op, vec![r], None)?
                    }
                };
                let setter = format!("{}=", name);
                self.call_method(&rv, &setter, vec![val.clone()], None)?;
                return Ok(val);
            }
        };
        let val = match op {
            "||" if cur.truthy() => return Ok(cur),
            "&&" if !cur.truthy() => return Ok(cur),
            "||" | "&&" => self.eval(rhs)?,
            _ => {
                let r = self.eval(rhs)?;
                self.call_method(&cur, op, vec![r], None)?
            }
        };
        self.apply_setter(&setter, val.clone())?;
        Ok(val)
    }

    fn eval_for(&mut self, vars: &[VarRef], iter: &Node, body: &[Node]) -> R<Value> {
        let coll = self.eval(iter)?;
        let run = |it: &mut Interp, item: Value| -> R<Option<Value>> {
            it.tick()?;
            if vars.len() == 1 {
                it.write_var(&vars[0], item);
            } else {
                let items = match &item {
                    Value::Array(a) => a.borrow().clone(),
                    other => vec![other.clone()],
                };
                for (i, v) in vars.iter().enumerate() {
                    it.write_var(v, items.get(i).cloned().unwrap_or(Value::Nil));
                }
            }
            match it.eval_body(body) {
                Ok(_) | Err(Flow::Next(_)) => Ok(None),
                Err(Flow::Break(v)) => Ok(Some(v)),
                Err(e) => Err(e),
            }
        };
        match &coll {
            Value::Array(a) => {
                let mut i = 0;
                loop {
                    let item = match a.borrow().get(i) {
                        Some(v) => v.clone(),
                        None => break,
                    };
                    if let Some(v) = run(self, item)? {
                        return Ok(v);
                    }
                    i += 1;
                }
            }
            Value::Range(r) => {
                if let (Value::Int(lo), hi) = (&r.start, &r.end) {
                    let hi = builtins::range::int_end(hi, r.excl)?;
                    let mut i = *lo;
                    while hi.is_none_or(|h| i <= h) {
                        if let Some(v) = run(self, Value::Int(i))? {
                            return Ok(v);
                        }
                        match i.checked_add(1) {
                            Some(n) => i = n,
                            None => break,
                        }
                    }
                } else {
                    for item in builtins::iter_values(self, &coll)? {
                        if let Some(v) = run(self, item)? {
                            return Ok(v);
                        }
                    }
                }
            }
            _ => {
                for item in builtins::iter_values(self, &coll)? {
                    if let Some(v) = run(self, item)? {
                        return Ok(v);
                    }
                }
            }
        }
        Ok(coll)
    }

    /// Renders another template (a partial) with `locals` as its host variables.
    pub fn render_partial(&mut self, path: &str, source: &str, locals: Vec<(Arc<str>, Value)>) -> R<String> {
        if self.render_depth + 1 > MAX_RENDER_DEPTH {
            return fail(LIMIT_ERROR, format!("render nesting deeper than {}", MAX_RENDER_DEPTH));
        }
        let mut names: Vec<String> = locals.iter().map(|(k, _)| k.to_string()).collect();
        names.sort();
        names.dedup();
        let program = match crate::compile_program(source, &names) {
            Ok(p) => p,
            Err(Flow::Err(mut e)) => {
                if e.filename.is_none() {
                    e.filename = Some(path.to_string());
                }
                return Err(Flow::Err(e));
            }
            Err(other) => return Err(other),
        };
        let mut child = Interp::new(self.ctx.clone(), path, locals, self.render_depth + 1);
        child.depth = self.depth;
        match child.run(&program) {
            Ok(s) => Ok(s),
            Err(Flow::Err(mut e)) => {
                if e.filename.is_none() {
                    e.filename = Some(path.to_string());
                }
                Err(Flow::Err(e))
            }
            Err(_) => Err(rt_err(LIMIT_ERROR, "invalid break")),
        }
    }
}

/// Turns a `break` out of a block into the value of the call that received the block.
pub fn catch_break(r: R<Value>) -> R<Value> {
    match r {
        Err(Flow::Break(v)) => Ok(v),
        other => other,
    }
}

fn bind_params(params: &[Param], args: Vec<Value>, splat: bool, frame: &mut HashMap<Arc<str>, Value>) {
    let vals = if splat && args.len() == 1 {
        match &args[0] {
            Value::Array(a) => a.borrow().clone(),
            _ => args,
        }
    } else {
        args
    };
    for (i, p) in params.iter().enumerate() {
        bind_param(p, vals.get(i).cloned().unwrap_or(Value::Nil), frame);
    }
}

fn bind_param(p: &Param, v: Value, frame: &mut HashMap<Arc<str>, Value>) {
    match p {
        Param::Var(r) => {
            frame.insert(r.name.clone(), v);
        }
        Param::Destructure(subs) => {
            let items = match &v {
                Value::Array(a) => a.borrow().clone(),
                other => vec![other.clone()],
            };
            for (i, s) in subs.iter().enumerate() {
                bind_param(s, items.get(i).cloned().unwrap_or(Value::Nil), frame);
            }
        }
    }
}

/// Builds a range, checking that the endpoints can be compared.
pub fn make_range(a: Value, b: Value, excl: bool) -> R<Value> {
    let ok = match (&a, &b) {
        (Value::Nil, _) | (_, Value::Nil) => true,
        _ => a.compare(&b).is_some(),
    };
    if !ok {
        return fail(ARGUMENT_ERROR, "bad value for range");
    }
    Ok(Value::range(a, b, excl))
}
