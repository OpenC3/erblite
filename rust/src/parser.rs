// Copyright 2026 OpenC3, Inc.
// All Rights Reserved.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
// See LICENSE.md for more details.

//! Recursive-descent parser: tokens -> AST, with Ruby-style local variable scoping.

use crate::ast::{Arg, BlockNode, Kind, Node, Param, Target, VarRef};
use crate::error::{Flow, RtError, LIMIT_ERROR};
use crate::lexer::{syntax_error, StrPart, Tok, Token};
use std::sync::Arc;

/// Deepest allowed syntactic nesting (each nested expression or block counts).
const MAX_PARSE_DEPTH: u32 = 2000;

type PR<T> = Result<T, Flow>;

struct Scope {
    vars: Vec<(Arc<str>, bool)>,
}

pub struct Parser {
    toks: Vec<Token>,
    pos: usize,
    scopes: Vec<Scope>,
    loop_depth: u32,
    no_do: bool,
    no_cmd: bool,
    depth: u32,
}

/// Parses a whole template token stream into a list of statements. Host variable names
/// are locals that already exist when parsing starts (as with `result_with_hash`).
pub fn parse_program(toks: Vec<Token>, host_names: &[String]) -> PR<Vec<Node>> {
    let vars = host_names.iter().map(|n| (Arc::from(n.as_str()), false)).collect();
    let mut p = Parser {
        toks,
        pos: 0,
        scopes: vec![Scope { vars }],
        loop_depth: 0,
        no_do: false,
        no_cmd: false,
        depth: 0,
    };
    let body = p.parse_statements(&|t| matches!(t, Tok::Eof))?;
    if !matches!(p.peek().tok, Tok::Eof) {
        return Err(p.unexpected());
    }
    Ok(body)
}

fn is_binop(t: &Tok) -> Option<(&'static str, u8)> {
    let Tok::Op(o) = t else { return None };
    let prec = match *o {
        "||" => 1,
        "&&" => 2,
        "==" | "!=" | "<=>" | "===" => 3,
        "<" | "<=" | ">" | ">=" => 4,
        "|" | "^" => 5,
        "&" => 6,
        "<<" | ">>" => 7,
        "+" | "-" => 8,
        "*" | "/" | "%" => 9,
        _ => return None,
    };
    Some((o, prec))
}

const OP_ASSIGN: &[&str] = &[
    "+=", "-=", "*=", "/=", "%=", "**=", "||=", "&&=", "<<=", ">>=", "|=", "&=", "^=",
];

impl Parser {
    fn peek(&self) -> &Token {
        // The stream always ends with Eof, and we never advance past it.
        &self.toks[self.pos.min(self.toks.len() - 1)]
    }

    fn peek_at(&self, n: usize) -> &Token {
        &self.toks[(self.pos + n).min(self.toks.len() - 1)]
    }

    fn advance(&mut self) -> Token {
        let t = self.peek().clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn line(&self) -> u32 {
        self.peek().line
    }

    fn is_op(&self, op: &str) -> bool {
        matches!(&self.peek().tok, Tok::Op(o) if *o == op)
    }

    fn is_kw(&self, kw: &str) -> bool {
        matches!(&self.peek().tok, Tok::Kw(k) if *k == kw)
    }

    fn unexpected(&self) -> Flow {
        let t = self.peek();
        let desc = match &t.tok {
            Tok::Eof => "unexpected end of input".to_string(),
            Tok::Newline => "unexpected end of line".to_string(),
            Tok::Op(o) => format!("unexpected '{}'", o),
            Tok::Kw(k) => format!("unexpected '{}'", k),
            Tok::Reserved(w) => format!("'{}' is not supported", w),
            Tok::Ident(w) | Tok::Const(w) => format!("unexpected identifier '{}'", w),
            Tok::Label(w) | Tok::StrLabel(w) => format!("unexpected label '{}:'", w),
            Tok::Text(_) => "unexpected template text inside an expression".to_string(),
            Tok::OutStart => "unexpected '<%='".to_string(),
            Tok::OutEnd => "unexpected end of tag".to_string(),
            _ => "unexpected token".to_string(),
        };
        syntax_error(desc, t.line)
    }

    fn expect_op(&mut self, op: &str) -> PR<()> {
        if self.is_op(op) {
            self.advance();
            Ok(())
        } else {
            Err(self.unexpected())
        }
    }

    fn expect_kw(&mut self, kw: &str) -> PR<()> {
        if self.is_kw(kw) {
            self.advance();
            Ok(())
        } else {
            Err(self.unexpected())
        }
    }

    fn skip_newlines(&mut self) {
        while matches!(self.peek().tok, Tok::Newline) {
            self.advance();
        }
    }

    fn skip_terms(&mut self) {
        while matches!(self.peek().tok, Tok::Newline) || self.is_op(";") {
            self.advance();
        }
    }

    fn limit_error(&self) -> Flow {
        Flow::Err(Box::new(RtError {
            kind: LIMIT_ERROR,
            message: "template nesting too deep".into(),
            line: Some(self.line()),
            filename: None,
        }))
    }

    fn enter(&mut self) -> PR<()> {
        self.depth += 1;
        if crate::stack::exhausted() {
            return Err(Flow::Err(Box::new(RtError {
                kind: LIMIT_ERROR,
                message: crate::stack::STACK_LIMIT_MESSAGE.into(),
                line: Some(self.line()),
                filename: None,
            })));
        }
        if self.depth > MAX_PARSE_DEPTH {
            return Err(Flow::Err(Box::new(RtError {
                kind: LIMIT_ERROR,
                message: "template nesting too deep".into(),
                line: Some(self.line()),
                filename: None,
            })));
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    // ----- scopes -------------------------------------------------------------------------

    fn lookup(&self, name: &str) -> Option<VarRef> {
        for (depth, scope) in self.scopes.iter().enumerate().rev() {
            if let Some((n, is_param)) = scope.vars.iter().find(|(n, _)| &**n == name) {
                let _ = is_param;
                return Some(VarRef { name: n.clone(), depth });
            }
        }
        None
    }

    fn declare(&mut self, name: &str) -> VarRef {
        if let Some(v) = self.lookup(name) {
            return v;
        }
        let depth = self.scopes.len() - 1;
        let rc: Arc<str> = Arc::from(name);
        if let Some(scope) = self.scopes.last_mut() {
            scope.vars.push((rc.clone(), false));
        }
        VarRef { name: rc, depth }
    }

    fn declare_param(&mut self, name: &str) -> VarRef {
        let depth = self.scopes.len() - 1;
        let rc: Arc<str> = Arc::from(name);
        if let Some(scope) = self.scopes.last_mut() {
            scope.vars.retain(|(n, _)| &**n != name);
            scope.vars.push((rc.clone(), true));
        }
        VarRef { name: rc, depth }
    }

    // ----- statements ---------------------------------------------------------------------

    #[inline(never)]
    fn parse_statements(&mut self, enders: &dyn Fn(&Tok) -> bool) -> PR<Vec<Node>> {
        let mut out = Vec::new();
        loop {
            self.skip_terms();
            let t = &self.peek().tok;
            if enders(t) || matches!(t, Tok::Eof) {
                break;
            }
            let stmt = self.parse_statement()?;
            let self_terminated = matches!(stmt.kind, Kind::Text(_) | Kind::Output(_));
            out.push(stmt);
            if self_terminated {
                continue;
            }
            let t = &self.peek().tok;
            let ok = matches!(t, Tok::Newline | Tok::Text(_) | Tok::OutStart | Tok::Eof)
                || matches!(t, Tok::Op(";"))
                || enders(t);
            if !ok {
                return Err(self.unexpected());
            }
        }
        Ok(out)
    }

    #[inline(never)]
    fn parse_statement(&mut self) -> PR<Node> {
        self.enter()?;
        let r = self.parse_statement_inner();
        self.leave();
        r
    }

    #[inline(never)]
    fn parse_statement_inner(&mut self) -> PR<Node> {
        let line = self.line();
        match &self.peek().tok {
            Tok::Text(s) => {
                let s: Arc<str> = Arc::from(s.as_str());
                self.advance();
                return Ok(Node::new(Kind::Text(s), line));
            }
            Tok::OutStart => {
                self.advance();
                let saved = (self.no_do, self.no_cmd);
                self.no_do = false;
                self.no_cmd = false;
                let body = self.parse_statements(&|t| matches!(t, Tok::OutEnd))?;
                (self.no_do, self.no_cmd) = saved;
                if !matches!(self.peek().tok, Tok::OutEnd) {
                    return Err(self.unexpected());
                }
                self.advance();
                return Ok(Node::new(
                    Kind::Output(Box::new(Node::new(Kind::Seq(body), line))),
                    line,
                ));
            }
            _ => {}
        }
        let mut node = self.parse_expr_statement()?;
        let mut chain = 0;
        let r = loop {
            let line = self.line();
            let kw = match &self.peek().tok {
                Tok::Kw(k @ ("if" | "unless" | "while" | "until")) => *k,
                _ => break Ok(node),
            };
            self.depth += 1;
            chain += 1;
            if self.depth > MAX_PARSE_DEPTH {
                break Err(self.limit_error());
            }
            self.advance();
            let cond = match self.parse_expr() {
                Ok(c) => c,
                Err(e) => break Err(e),
            };
            node = match kw {
                "if" => Node::new(Kind::If(Box::new(cond), vec![node], vec![]), line),
                "unless" => Node::new(Kind::If(Box::new(cond), vec![], vec![node]), line),
                _ => Node::new(
                    Kind::While {
                        cond: Box::new(cond),
                        body: vec![node],
                        until: kw == "until",
                    },
                    line,
                ),
            };
        };
        self.depth -= chain;
        r
    }

    fn can_start_value(&self) -> bool {
        match &self.peek().tok {
            Tok::Newline | Tok::Eof | Tok::OutEnd | Tok::Text(_) | Tok::OutStart => false,
            Tok::Op(o) => !matches!(
                *o,
                ";" | "}" | ")" | "]" | "," | "=>" | "?" | ":" | "|" | "||" | "&&" | "."
            ),
            Tok::Kw(k) => matches!(
                *k,
                "nil" | "true" | "false" | "if" | "unless" | "while" | "until" | "case" | "for" | "not"
            ),
            _ => true,
        }
    }

    #[inline(never)]
    fn parse_expr_statement(&mut self) -> PR<Node> {
        // multiple assignment: a, b = ...
        if let Tok::Ident(_) = &self.peek().tok {
            if matches!(self.peek_at(1).tok, Tok::Op(",")) {
                let mut n = 0;
                let mut ok = false;
                loop {
                    if !matches!(self.peek_at(n).tok, Tok::Ident(_)) {
                        break;
                    }
                    match self.peek_at(n + 1).tok {
                        Tok::Op(",") => n += 2,
                        Tok::Op("=") => {
                            ok = true;
                            break;
                        }
                        _ => break,
                    }
                }
                if ok {
                    return self.parse_multi_assign();
                }
            }
        }
        let node = self.parse_expr()?;
        if !self.is_op(",") {
            return Ok(node);
        }
        // `a = 1, 2` assigns an Array
        match node {
            Node {
                kind: Kind::Assign(var, first),
                line,
            } => {
                let mut items = vec![*first];
                while self.is_op(",") {
                    self.advance();
                    self.skip_newlines();
                    items.push(self.parse_assign()?);
                }
                Ok(Node::new(
                    Kind::Assign(var, Box::new(Node::new(Kind::Array(items), line))),
                    line,
                ))
            }
            other => Ok(other),
        }
    }

    #[inline(never)]
    fn parse_multi_assign(&mut self) -> PR<Node> {
        let line = self.line();
        let mut names = Vec::new();
        loop {
            if let Tok::Ident(name) = self.advance().tok {
                names.push(name);
            }
            if self.is_op(",") {
                self.advance();
                continue;
            }
            break;
        }
        self.expect_op("=")?;
        let targets: Vec<VarRef> = names.iter().map(|n| self.declare(n)).collect();
        let mut values = vec![self.parse_assign()?];
        while self.is_op(",") {
            self.advance();
            self.skip_newlines();
            values.push(self.parse_assign()?);
        }
        Ok(Node::new(Kind::MultiAssign(targets, values), line))
    }

    // ----- expressions --------------------------------------------------------------------

    /// expr := not_expr { (and | or) not_expr }
    #[inline(never)]
    fn parse_expr(&mut self) -> PR<Node> {
        self.enter()?;
        let r = self.parse_expr_inner();
        self.leave();
        r
    }

    #[inline(never)]
    fn parse_expr_inner(&mut self) -> PR<Node> {
        let lhs = self.parse_not_expr()?;
        let mut chain = 0;
        let r = self.parse_expr_chain(lhs, &mut chain);
        self.depth -= chain;
        r
    }

    #[inline(never)]
    fn parse_expr_chain(&mut self, mut lhs: Node, chain: &mut u32) -> PR<Node> {
        loop {
            if self.is_kw("and") || self.is_kw("or") {
                self.depth += 1;
                *chain += 1;
                if self.depth > MAX_PARSE_DEPTH {
                    return Err(self.limit_error());
                }
            }
            let line = self.line();
            if self.is_kw("and") {
                self.advance();
                self.skip_newlines();
                let rhs = self.parse_not_expr()?;
                lhs = Node::new(Kind::And(Box::new(lhs), Box::new(rhs)), line);
            } else if self.is_kw("or") {
                self.advance();
                self.skip_newlines();
                let rhs = self.parse_not_expr()?;
                lhs = Node::new(Kind::Or(Box::new(lhs), Box::new(rhs)), line);
            } else {
                break;
            }
        }
        Ok(lhs)
    }

    #[inline(never)]
    fn parse_not_expr(&mut self) -> PR<Node> {
        if self.is_kw("not") {
            let line = self.line();
            self.advance();
            self.enter()?;
            let inner = self.parse_not_expr();
            self.leave();
            return Ok(Node::new(Kind::Not(Box::new(inner?)), line));
        }
        if matches!(self.peek().tok, Tok::Kw("break") | Tok::Kw("next")) {
            return self.parse_break_next();
        }
        self.parse_assign()
    }

    #[inline(never)]
    fn parse_break_next(&mut self) -> PR<Node> {
        let line = self.line();
        let is_break = self.is_kw("break");
        self.advance();
        if self.loop_depth == 0 {
            let w = if is_break { "break" } else { "next" };
            return Err(syntax_error(format!("Invalid {}", w), line));
        }
        let value = if self.can_start_value()
            && !self.is_kw("if")
            && !self.is_kw("unless")
            && !self.is_kw("while")
            && !self.is_kw("until")
        {
            Some(Box::new(self.parse_assign()?))
        } else {
            None
        };
        Ok(Node::new(
            if is_break {
                Kind::Break(value)
            } else {
                Kind::Next(value)
            },
            line,
        ))
    }

    #[inline(never)]
    fn parse_assign(&mut self) -> PR<Node> {
        let line = self.line();
        // Fast path: IDENT = ... / IDENT op= ...
        if let Tok::Ident(name) = &self.peek().tok {
            if let Tok::Op(op) = self.peek_at(1).tok {
                if op == "=" {
                    let name = name.clone();
                    self.advance();
                    self.advance();
                    self.skip_newlines();
                    let var = self.declare(&name);
                    let rhs = self.parse_assign_rhs()?;
                    return Ok(Node::new(Kind::Assign(var, Box::new(rhs)), line));
                }
                if OP_ASSIGN.contains(&op) {
                    let name = name.clone();
                    self.advance();
                    self.advance();
                    self.skip_newlines();
                    let var = self.declare(&name);
                    let rhs = self.parse_assign_rhs()?;
                    let bin = &op[..op.len() - 1];
                    return Ok(Node::new(
                        Kind::OpAssign(Target::Var(var), Arc::from(bin), Box::new(rhs)),
                        line,
                    ));
                }
            }
        }
        if let Tok::Const(_) = &self.peek().tok {
            if let Tok::Op(op) = self.peek_at(1).tok {
                if op == "=" || OP_ASSIGN.contains(&op) {
                    return Err(syntax_error("constant assignment is not supported", line));
                }
            }
        }
        let lhs = self.parse_ternary()?;
        let op = match &self.peek().tok {
            Tok::Op(o) if *o == "=" || OP_ASSIGN.contains(o) => *o,
            _ => return Ok(lhs),
        };
        let op_line = self.line();
        self.advance();
        self.skip_newlines();
        let rhs = self.parse_assign_rhs()?;
        let Node { kind, line: lline } = lhs;
        let target = match kind {
            Kind::Call {
                recv,
                name,
                args,
                block: None,
                safe: false,
            } if &*name == "[]" => Target::Index(recv, args),
            Kind::Call {
                recv,
                name,
                args,
                block: None,
                safe: false,
            } if args.is_empty() && name.chars().next().is_some_and(|c| c.is_ascii_lowercase() || c == '_') => {
                Target::Attr(recv, name)
            }
            Kind::Var(v) => Target::Var(v),
            Kind::IdentCall {
                name,
                has_args: false,
                block: None,
                ..
            } => Target::Var(self.declare(&name)),
            _ => return Err(syntax_error("unexpected '=': invalid assignment target", op_line)),
        };
        if op == "=" {
            return Ok(match target {
                Target::Index(recv, args) => Node::new(Kind::IndexAssign(recv, args, Box::new(rhs)), op_line),
                Target::Attr(recv, name) => Node::new(Kind::AttrAssign(recv, name, Box::new(rhs)), op_line),
                Target::Var(v) => Node::new(Kind::Assign(v, Box::new(rhs)), lline),
            });
        }
        let bin = &op[..op.len() - 1];
        Ok(Node::new(
            Kind::OpAssign(target, Arc::from(bin), Box::new(rhs)),
            op_line,
        ))
    }

    #[inline(never)]
    fn parse_assign_rhs(&mut self) -> PR<Node> {
        self.enter()?;
        let r = if self.is_kw("not") {
            self.parse_not_expr()
        } else {
            self.parse_assign()
        };
        self.leave();
        r
    }

    #[inline(never)]
    fn parse_ternary(&mut self) -> PR<Node> {
        let cond = self.parse_range()?;
        if !self.is_op("?") {
            return Ok(cond);
        }
        let line = self.line();
        self.advance();
        self.skip_newlines();
        // `c ? x: y` / `c ? "a": "b"`: the label's colon is the ternary colon
        let a = match &self.peek().tok {
            Tok::Label(name) => {
                let (name, l) = (name.clone(), self.line());
                self.advance();
                match self.lookup(&name) {
                    Some(v) => Node::new(Kind::Var(v), l),
                    None => Node::new(
                        Kind::IdentCall {
                            name: Arc::from(name),
                            args: vec![],
                            has_args: false,
                            block: None,
                        },
                        l,
                    ),
                }
            }
            Tok::StrLabel(s) => {
                let (s, l) = (s.clone(), self.line());
                self.advance();
                Node::new(Kind::Str(Arc::from(s)), l)
            }
            _ => {
                self.enter()?;
                let a = self.parse_ternary_branch();
                self.leave();
                let a = a?;
                self.skip_newlines();
                self.expect_op(":")?;
                a
            }
        };
        self.skip_newlines();
        self.enter()?;
        let b = self.parse_ternary_branch();
        self.leave();
        Ok(Node::new(Kind::If(Box::new(cond), vec![a], vec![b?]), line))
    }

    #[inline(never)]
    fn parse_ternary_branch(&mut self) -> PR<Node> {
        if let Tok::Ident(_) = &self.peek().tok {
            if matches!(self.peek_at(1).tok, Tok::Op("=")) {
                return self.parse_assign();
            }
        }
        self.parse_ternary()
    }

    fn operand_follows(&self) -> bool {
        match &self.peek().tok {
            Tok::Newline | Tok::Eof | Tok::OutEnd | Tok::Text(_) | Tok::OutStart => false,
            Tok::Op(o) => matches!(*o, "(" | "[" | "-" | "!" | "~" | "{" | "::"),
            Tok::Kw(k) => matches!(
                *k,
                "nil" | "true" | "false" | "if" | "unless" | "case" | "while" | "until" | "for"
            ),
            Tok::Label(_) | Tok::StrLabel(_) => false,
            _ => true,
        }
    }

    #[inline(never)]
    fn parse_range(&mut self) -> PR<Node> {
        let line = self.line();
        let lhs = if self.is_op("..") || self.is_op("...") {
            None
        } else {
            Some(self.parse_binary(1)?)
        };
        if self.is_op("..") || self.is_op("...") {
            let excl = self.is_op("...");
            self.advance();
            let rhs = if self.operand_follows() {
                Some(Box::new(self.parse_binary(1)?))
            } else {
                None
            };
            if lhs.is_none() && rhs.is_none() {
                return Err(self.unexpected());
            }
            return Ok(Node::new(Kind::Range(lhs.map(Box::new), rhs, excl), line));
        }
        match lhs {
            Some(n) => Ok(n),
            None => Err(self.unexpected()),
        }
    }

    #[inline(never)]
    fn parse_binary(&mut self, min_prec: u8) -> PR<Node> {
        let lhs = self.parse_uminus()?;
        let mut chain = 0;
        let r = self.parse_binary_chain(lhs, min_prec, &mut chain);
        self.depth -= chain;
        r
    }

    #[inline(never)]
    fn parse_binary_chain(&mut self, mut lhs: Node, min_prec: u8, chain: &mut u32) -> PR<Node> {
        while let Some((op, prec)) = is_binop(&self.peek().tok) {
            if prec < min_prec {
                break;
            }
            self.depth += 1;
            *chain += 1;
            if self.depth > MAX_PARSE_DEPTH {
                return Err(self.limit_error());
            }
            let line = self.line();
            self.advance();
            self.skip_newlines();
            self.enter()?;
            let rhs = self.parse_binary(prec + 1);
            self.leave();
            let rhs = rhs?;
            lhs = match op {
                "&&" => Node::new(Kind::And(Box::new(lhs), Box::new(rhs)), line),
                "||" => Node::new(Kind::Or(Box::new(lhs), Box::new(rhs)), line),
                "!=" => Node::new(Kind::Not(Box::new(call(lhs, "==", vec![rhs], line))), line),
                _ => call(lhs, op, vec![rhs], line),
            };
            if prec == 3 {
                if let Some((_, 3)) = is_binop(&self.peek().tok) {
                    return Err(self.unexpected());
                }
            }
        }
        Ok(lhs)
    }

    #[inline(never)]
    fn parse_uminus(&mut self) -> PR<Node> {
        if self.is_op("-") {
            let line = self.line();
            self.advance();
            self.enter()?;
            let inner = self.parse_uminus();
            self.leave();
            return Ok(call(inner?, "-@", vec![], line));
        }
        self.parse_pow()
    }

    #[inline(never)]
    fn parse_pow(&mut self) -> PR<Node> {
        let neg_lit = self.peek().neg_lit;
        let base = self.parse_unary()?;
        if !self.is_op("**") {
            return Ok(base);
        }
        let line = self.line();
        self.advance();
        self.skip_newlines();
        self.enter()?;
        let rhs = self.parse_uminus();
        self.leave();
        let rhs = rhs?;
        if neg_lit {
            let positive = match base.kind {
                Kind::Int(i) => match i.checked_neg() {
                    Some(p) => Some(Kind::Int(p)),
                    None => Some(Kind::IntOverflow),
                },
                Kind::Float(f) => Some(Kind::Float(-f)),
                Kind::IntOverflow => Some(Kind::IntOverflow),
                other => {
                    return Ok(call(Node::new(other, base.line), "**", vec![rhs], line));
                }
            };
            if let Some(k) = positive {
                let pow = call(Node::new(k, base.line), "**", vec![rhs], line);
                return Ok(call(pow, "-@", vec![], line));
            }
        }
        Ok(call(base, "**", vec![rhs], line))
    }

    #[inline(never)]
    fn parse_unary(&mut self) -> PR<Node> {
        let line = self.line();
        let (op, name) = match &self.peek().tok {
            Tok::Op("!") => ("!", ""),
            Tok::Op("~") => ("~", "~"),
            Tok::Op("+") => ("+", "+@"),
            Tok::Op("-") => return self.parse_uminus(),
            _ => return self.parse_postfix(),
        };
        self.advance();
        self.enter()?;
        let inner = self.parse_unary();
        self.leave();
        let inner = inner?;
        Ok(if op == "!" {
            Node::new(Kind::Not(Box::new(inner)), line)
        } else {
            call(inner, name, vec![], line)
        })
    }

    #[inline(never)]
    fn parse_postfix(&mut self) -> PR<Node> {
        let node = self.parse_primary()?;
        let mut chain = 0;
        let r = self.parse_postfix_chain(node, &mut chain);
        self.depth -= chain;
        r
    }

    /// Each link of a call chain counts as one nesting level (it nests the AST one deeper).
    #[inline(never)]
    fn parse_postfix_chain(&mut self, mut node: Node, chain: &mut u32) -> PR<Node> {
        loop {
            if matches!(self.peek().tok, Tok::Op("." | "&." | "::")) || (self.is_op("[") && !self.peek().space_before) {
                self.depth += 1;
                *chain += 1;
                if self.depth > MAX_PARSE_DEPTH {
                    return Err(self.limit_error());
                }
            }
            let t = self.peek().clone();
            match &t.tok {
                Tok::Op(o @ ("." | "&.")) => {
                    let safe = *o == "&.";
                    self.advance();
                    self.skip_newlines();
                    let nt = self.advance();
                    let name: String = match nt.tok {
                        Tok::Ident(n) | Tok::Const(n) | Tok::Reserved(n) => n,
                        Tok::Kw(k) => k.to_string(),
                        Tok::Op("[") if self.is_op("]") => {
                            self.advance();
                            if self.is_op("=") && !self.peek().space_before {
                                self.advance();
                                "[]=".to_string()
                            } else {
                                "[]".to_string()
                            }
                        }
                        Tok::Op(
                            o @ ("+" | "-" | "*" | "/" | "%" | "**" | "==" | "!=" | "<" | ">" | "<=" | ">=" | "<=>"
                            | "<<" | ">>" | "&" | "|" | "^" | "!" | "~" | "==="),
                        ) => o.to_string(),
                        Tok::Label(n) => {
                            // `obj.meth:` is never valid here
                            return Err(syntax_error(format!("unexpected label '{}:'", n), nt.line));
                        }
                        _ => return Err(syntax_error("expected a method name after '.'", nt.line)),
                    };
                    let line = nt.line;
                    let (args, has_args, cmd) = self.parse_call_args()?;
                    let _ = has_args;
                    let block = self.parse_block_opt(cmd)?;
                    node = Node::new(
                        Kind::Call {
                            recv: Box::new(node),
                            name: Arc::from(name),
                            args,
                            block,
                            safe,
                        },
                        line,
                    );
                    if cmd {
                        break;
                    }
                }
                Tok::Op("::") => {
                    self.advance();
                    let nt = self.advance();
                    match nt.tok {
                        Tok::Const(n) => {
                            if self.is_op("(") && !self.peek().space_before {
                                let (args, _, _) = self.parse_call_args()?;
                                node = Node::new(
                                    Kind::Call {
                                        recv: Box::new(node),
                                        name: Arc::from(n),
                                        args,
                                        block: None,
                                        safe: false,
                                    },
                                    nt.line,
                                );
                            } else {
                                node = Node::new(Kind::ScopedConst(Box::new(node), Arc::from(n)), nt.line);
                            }
                        }
                        Tok::Ident(n) => {
                            let (args, _, cmd) = self.parse_call_args()?;
                            let block = self.parse_block_opt(cmd)?;
                            node = Node::new(
                                Kind::Call {
                                    recv: Box::new(node),
                                    name: Arc::from(n),
                                    args,
                                    block,
                                    safe: false,
                                },
                                nt.line,
                            );
                        }
                        _ => return Err(syntax_error("expected a constant after '::'", nt.line)),
                    }
                }
                Tok::Op("[") if !t.space_before => {
                    let line = t.line;
                    self.advance();
                    let args = self.parse_arg_list("]")?;
                    node = Node::new(
                        Kind::Call {
                            recv: Box::new(node),
                            name: Arc::from("[]"),
                            args,
                            block: None,
                            safe: false,
                        },
                        line,
                    );
                }
                _ => break,
            }
        }
        Ok(node)
    }

    /// Parses `(args)` or command arguments after a method name.
    /// Returns (args, whether args were given, whether they were command args).
    #[inline(never)]
    fn parse_call_args(&mut self) -> PR<(Vec<Arg>, bool, bool)> {
        let t = self.peek();
        if matches!(t.tok, Tok::Op("(")) && !t.space_before {
            self.advance();
            let args = self.parse_arg_list(")")?;
            return Ok((args, true, false));
        }
        if self.command_arg_start() {
            let args = self.parse_command_args()?;
            return Ok((args, true, true));
        }
        Ok((Vec::new(), false, false))
    }

    fn command_arg_start(&self) -> bool {
        if self.no_cmd {
            return false;
        }
        let t = self.peek();
        if !t.space_before {
            return false;
        }
        match &t.tok {
            Tok::Op("(" | "[" | "!") => true,
            Tok::Op("&") => matches!(self.peek_at(1).tok, Tok::Sym(_)),
            Tok::Op("-") => {
                let n = self.peek_at(1);
                matches!(n.tok, Tok::Int(_) | Tok::BigInt | Tok::Float(_)) && !n.space_before
            }
            Tok::Int(_) | Tok::BigInt | Tok::Float(_) | Tok::Str(_) | Tok::Sym(_) | Tok::DSym(_) => true,
            Tok::Words(_) | Tok::Symbols(_) | Tok::Label(_) | Tok::StrLabel(_) | Tok::Ident(_) | Tok::Const(_) => true,
            Tok::Kw(k) => matches!(*k, "nil" | "true" | "false"),
            _ => false,
        }
    }

    #[inline(never)]
    fn parse_command_args(&mut self) -> PR<Vec<Arg>> {
        let saved = self.no_cmd;
        self.no_cmd = true;
        let mut args = Vec::new();
        let r = (|| -> PR<()> {
            loop {
                self.parse_one_arg(&mut args)?;
                if self.is_op(",") {
                    self.advance();
                    self.skip_newlines();
                    continue;
                }
                break;
            }
            Ok(())
        })();
        self.no_cmd = saved;
        r?;
        Ok(args)
    }

    /// Parses arguments up to the closing bracket `close` (already past the opener).
    #[inline(never)]
    fn parse_arg_list(&mut self, close: &str) -> PR<Vec<Arg>> {
        let saved = (self.no_do, self.no_cmd);
        self.no_do = false;
        self.no_cmd = false;
        let mut args = Vec::new();
        let r = (|| -> PR<()> {
            loop {
                self.skip_newlines();
                if self.is_op(close) {
                    self.advance();
                    return Ok(());
                }
                self.parse_one_arg(&mut args)?;
                self.skip_newlines();
                if self.is_op(",") {
                    self.advance();
                    continue;
                }
                self.skip_newlines();
                if self.is_op(close) {
                    self.advance();
                    return Ok(());
                }
                return Err(self.unexpected());
            }
        })();
        (self.no_do, self.no_cmd) = saved;
        r?;
        Ok(args)
    }

    #[inline(never)]
    fn parse_one_arg(&mut self, args: &mut Vec<Arg>) -> PR<()> {
        let line = self.line();
        if self.is_op("&") {
            self.advance();
            let t = self.advance();
            match t.tok {
                Tok::Sym(s) => {
                    args.push(Arg::BlockPass(Arc::from(s)));
                    return Ok(());
                }
                _ => return Err(syntax_error("only &:symbol block arguments are supported", t.line)),
            }
        }
        if let Tok::Label(name) | Tok::StrLabel(name) = &self.peek().tok {
            let key = Node::new(Kind::Sym(Arc::from(name.as_str())), line);
            self.advance();
            self.skip_newlines();
            let v = self.parse_arg_value()?;
            args.push(Arg::Pair(key, v));
            return Ok(());
        }
        let e = self.parse_arg_value()?;
        if self.is_op("=>") {
            self.advance();
            self.skip_newlines();
            let v = self.parse_arg_value()?;
            args.push(Arg::Pair(e, v));
        } else {
            args.push(Arg::Pos(e));
        }
        Ok(())
    }

    #[inline(never)]
    fn parse_arg_value(&mut self) -> PR<Node> {
        self.enter()?;
        let r = if self.is_kw("not") {
            self.parse_not_expr()
        } else {
            self.parse_assign()
        };
        self.leave();
        r
    }

    /// Parses an optional `{ }` / `do end` block after a call.
    #[inline(never)]
    fn parse_block_opt(&mut self, after_command_args: bool) -> PR<Option<Box<BlockNode>>> {
        if self.is_op("{") && !after_command_args {
            return Ok(Some(Box::new(self.parse_block(true)?)));
        }
        if self.is_kw("do") && !self.no_do {
            return Ok(Some(Box::new(self.parse_block(false)?)));
        }
        Ok(None)
    }

    #[inline(never)]
    fn parse_block(&mut self, brace: bool) -> PR<BlockNode> {
        self.enter()?;
        self.advance();
        self.scopes.push(Scope { vars: Vec::new() });
        self.loop_depth += 1;
        let saved = (self.no_do, self.no_cmd);
        self.no_do = false;
        self.no_cmd = false;
        let r = self.parse_block_inner(brace);
        (self.no_do, self.no_cmd) = saved;
        self.loop_depth -= 1;
        self.scopes.pop();
        self.leave();
        r
    }

    #[inline(never)]
    fn parse_block_inner(&mut self, brace: bool) -> PR<BlockNode> {
        self.skip_newlines();
        let mut params = Vec::new();
        let mut splat = false;
        if self.is_op("||") {
            self.advance();
        } else if self.is_op("|") {
            self.advance();
            loop {
                self.skip_newlines();
                if self.is_op("|") {
                    break;
                }
                params.push(self.parse_param()?);
                self.skip_newlines();
                if self.is_op(",") {
                    self.advance();
                    self.skip_newlines();
                    if self.is_op("|") {
                        splat = true;
                    }
                    continue;
                }
                break;
            }
            self.expect_op("|")?;
        }
        if params.len() > 1 {
            splat = true;
        }
        let body = if brace {
            let b = self.parse_statements(&|t| matches!(t, Tok::Op("}")))?;
            self.expect_op("}")?;
            b
        } else {
            let b = self.parse_statements(&|t| matches!(t, Tok::Kw("end")))?;
            self.expect_kw("end")?;
            b
        };
        Ok(BlockNode { params, body, splat })
    }

    #[inline(never)]
    fn parse_param(&mut self) -> PR<Param> {
        let t = self.advance();
        match t.tok {
            Tok::Ident(name) => Ok(Param::Var(self.declare_param(&name))),
            Tok::Op("(") => {
                let mut subs = Vec::new();
                loop {
                    subs.push(self.parse_param()?);
                    if self.is_op(",") {
                        self.advance();
                        continue;
                    }
                    break;
                }
                self.expect_op(")")?;
                Ok(Param::Destructure(subs))
            }
            _ => Err(syntax_error("unsupported block parameter", t.line)),
        }
    }

    // ----- primaries ----------------------------------------------------------------------

    #[inline(never)]
    fn parse_primary(&mut self) -> PR<Node> {
        let t = self.peek().clone();
        let line = t.line;
        match t.tok {
            Tok::Int(v) => {
                self.advance();
                Ok(Node::new(
                    match i64::try_from(v) {
                        Ok(i) => Kind::Int(i),
                        Err(_) => Kind::IntOverflow,
                    },
                    line,
                ))
            }
            Tok::BigInt => {
                self.advance();
                Ok(Node::new(Kind::IntOverflow, line))
            }
            Tok::Float(f) => {
                self.advance();
                Ok(Node::new(Kind::Float(f), line))
            }
            Tok::Str(mut parts) => {
                self.advance();
                // adjacent literals concatenate: "a" "b"
                while let Tok::Str(more) = &self.peek().tok {
                    parts.extend(more.iter().cloned());
                    self.advance();
                }
                self.string_node(merge_lits(parts), line, false)
            }
            Tok::DSym(parts) => {
                self.advance();
                self.string_node(parts, line, true)
            }
            Tok::Sym(s) => {
                self.advance();
                Ok(Node::new(Kind::Sym(Arc::from(s)), line))
            }
            Tok::Words(ws) => {
                self.advance();
                Ok(Node::new(
                    Kind::Array(
                        ws.into_iter()
                            .map(|w| Node::new(Kind::Str(Arc::from(w)), line))
                            .collect(),
                    ),
                    line,
                ))
            }
            Tok::Symbols(ws) => {
                self.advance();
                Ok(Node::new(
                    Kind::Array(
                        ws.into_iter()
                            .map(|w| Node::new(Kind::Sym(Arc::from(w)), line))
                            .collect(),
                    ),
                    line,
                ))
            }
            Tok::Op("[") => self.parse_array_literal(line),
            Tok::Op("{") => {
                self.advance();
                self.parse_hash_literal(line)
            }
            Tok::Op("(") => self.parse_paren(line),
            Tok::Ident(name) => {
                self.advance();
                self.parse_identifier(name, line)
            }
            Tok::Const(name) => self.parse_const(name, line),
            Tok::Kw("nil") => {
                self.advance();
                Ok(Node::new(Kind::Nil, line))
            }
            Tok::Kw("true") => {
                self.advance();
                Ok(Node::new(Kind::True, line))
            }
            Tok::Kw("false") => {
                self.advance();
                Ok(Node::new(Kind::False, line))
            }
            Tok::Kw("if") => self.parse_if(false),
            Tok::Kw("unless") => self.parse_if(true),
            Tok::Kw("while") => self.parse_while(false),
            Tok::Kw("until") => self.parse_while(true),
            Tok::Kw("for") => self.parse_for(),
            Tok::Kw("case") => self.parse_case(),
            Tok::Kw("break") | Tok::Kw("next") => self.parse_break_next(),
            _ => Err(self.unexpected()),
        }
    }

    #[inline(never)]
    fn parse_array_literal(&mut self, line: u32) -> PR<Node> {
        self.advance();
        let args = self.parse_arg_list("]")?;
        let mut items = Vec::new();
        let mut pairs = Vec::new();
        for a in args {
            match a {
                Arg::Pos(n) => {
                    if !pairs.is_empty() {
                        items.push(Node::new(Kind::Hash(std::mem::take(&mut pairs)), line));
                    }
                    items.push(n)
                }
                Arg::Pair(k, v) => pairs.push((k, v)),
                Arg::BlockPass(_) => return Err(syntax_error("unexpected block argument", line)),
            }
        }
        if !pairs.is_empty() {
            items.push(Node::new(Kind::Hash(pairs), line));
        }
        Ok(Node::new(Kind::Array(items), line))
    }

    #[inline(never)]
    fn parse_paren(&mut self, line: u32) -> PR<Node> {
        self.advance();
        let saved = (self.no_do, self.no_cmd);
        self.no_do = false;
        self.no_cmd = false;
        let body = self.parse_statements(&|t| matches!(t, Tok::Op(")")));
        (self.no_do, self.no_cmd) = saved;
        let body = body?;
        self.expect_op(")")?;
        Ok(match body.len() {
            0 => Node::new(Kind::Nil, line),
            _ => Node::new(Kind::Seq(body), line),
        })
    }

    #[inline(never)]
    fn parse_const(&mut self, name: String, line: u32) -> PR<Node> {
        self.advance();
        let n = self.peek();
        if matches!(n.tok, Tok::Op("(")) && !n.space_before {
            let (args, _, _) = self.parse_call_args()?;
            let block = self.parse_block_opt(false)?;
            return Ok(Node::new(
                Kind::IdentCall {
                    name: Arc::from(name),
                    args,
                    has_args: true,
                    block,
                },
                line,
            ));
        }
        Ok(Node::new(Kind::Const(Arc::from(name)), line))
    }

    #[inline(never)]
    fn parse_identifier(&mut self, name: String, line: u32) -> PR<Node> {
        let next = self.peek();
        let paren = matches!(next.tok, Tok::Op("(")) && !next.space_before;
        if !paren {
            if let Some(var) = self.lookup(&name) {
                return Ok(Node::new(Kind::Var(var), line));
            }
        }
        let (args, has_args, cmd) = self.parse_call_args()?;
        let block = self.parse_block_opt(cmd)?;
        Ok(Node::new(
            Kind::IdentCall {
                name: Arc::from(name),
                args,
                has_args,
                block,
            },
            line,
        ))
    }

    fn string_node(&mut self, parts: Vec<StrPart>, line: u32, sym: bool) -> PR<Node> {
        if let [StrPart::Lit(s)] = parts.as_slice() {
            let rc: Arc<str> = Arc::from(s.as_str());
            return Ok(Node::new(if sym { Kind::Sym(rc) } else { Kind::Str(rc) }, line));
        }
        let mut nodes = Vec::new();
        for p in parts {
            match p {
                StrPart::Lit(s) => nodes.push(Node::new(Kind::Str(Arc::from(s)), line)),
                StrPart::Code(toks) => nodes.push(self.parse_sub(toks, line)?),
            }
        }
        Ok(Node::new(if sym { Kind::DSym(nodes) } else { Kind::DStr(nodes) }, line))
    }

    /// Parses the tokens of a `#{...}` interpolation with the current scopes.
    #[inline(never)]
    fn parse_sub(&mut self, mut toks: Vec<Token>, line: u32) -> PR<Node> {
        let end_line = toks.last().map(|t| t.line).unwrap_or(line);
        toks.push(Token {
            tok: Tok::Eof,
            line: end_line,
            space_before: true,
            neg_lit: false,
        });
        let saved_toks = std::mem::replace(&mut self.toks, toks);
        let saved_pos = std::mem::replace(&mut self.pos, 0);
        let saved_flags = (self.no_do, self.no_cmd);
        self.no_do = false;
        self.no_cmd = false;
        let r = self.parse_statements(&|t| matches!(t, Tok::Eof));
        let at_end = matches!(self.peek().tok, Tok::Eof);
        let err = if r.is_ok() && !at_end {
            Some(self.unexpected())
        } else {
            None
        };
        self.toks = saved_toks;
        self.pos = saved_pos;
        (self.no_do, self.no_cmd) = saved_flags;
        if let Some(e) = err {
            return Err(e);
        }
        Ok(Node::new(Kind::Seq(r?), line))
    }

    #[inline(never)]
    fn parse_hash_literal(&mut self, line: u32) -> PR<Node> {
        let saved = (self.no_do, self.no_cmd);
        self.no_do = false;
        self.no_cmd = false;
        let r = self.parse_hash_entries();
        (self.no_do, self.no_cmd) = saved;
        Ok(Node::new(Kind::Hash(r?), line))
    }

    #[inline(never)]
    fn parse_hash_entries(&mut self) -> PR<Vec<(Node, Node)>> {
        let mut pairs = Vec::new();
        loop {
            self.skip_newlines();
            if self.is_op("}") {
                self.advance();
                return Ok(pairs);
            }
            let line = self.line();
            if let Tok::Label(name) | Tok::StrLabel(name) = &self.peek().tok {
                let key = Node::new(Kind::Sym(Arc::from(name.as_str())), line);
                self.advance();
                self.skip_newlines();
                let v = self.parse_arg_value()?;
                pairs.push((key, v));
            } else {
                let k = self.parse_arg_value()?;
                self.skip_newlines();
                self.expect_op("=>")?;
                self.skip_newlines();
                let v = self.parse_arg_value()?;
                pairs.push((k, v));
            }
            self.skip_newlines();
            if self.is_op(",") {
                self.advance();
                continue;
            }
            self.skip_newlines();
            if self.is_op("}") {
                self.advance();
                return Ok(pairs);
            }
            return Err(self.unexpected());
        }
    }

    #[inline(never)]
    fn parse_then(&mut self) -> PR<()> {
        if self.is_kw("then") {
            self.advance();
            return Ok(());
        }
        if matches!(self.peek().tok, Tok::Newline) || self.is_op(";") {
            self.skip_terms();
            if self.is_kw("then") {
                self.advance();
            }
            return Ok(());
        }
        Err(self.unexpected())
    }

    #[inline(never)]
    fn parse_cond(&mut self) -> PR<Node> {
        let saved = self.no_do;
        self.no_do = true;
        let r = self.parse_expr();
        self.no_do = saved;
        r
    }

    #[inline(never)]
    fn parse_if(&mut self, unless: bool) -> PR<Node> {
        let line = self.line();
        self.enter()?;
        self.advance();
        let r = (|| -> PR<Node> {
            let cond = self.parse_expr()?;
            self.parse_then()?;
            let enders = |t: &Tok| matches!(t, Tok::Kw("elsif") | Tok::Kw("else") | Tok::Kw("end"));
            let body = self.parse_statements(&enders)?;
            let els = if !unless && self.is_kw("elsif") {
                vec![self.parse_if(false)?]
            } else if self.is_kw("else") {
                self.advance();
                let b = self.parse_statements(&|t| matches!(t, Tok::Kw("end")))?;
                self.expect_kw("end")?;
                b
            } else {
                self.expect_kw("end")?;
                vec![]
            };
            Ok(if unless {
                Node::new(Kind::If(Box::new(cond), els, body), line)
            } else {
                Node::new(Kind::If(Box::new(cond), body, els), line)
            })
        })();
        self.leave();
        r
    }

    #[inline(never)]
    fn parse_loop_body(&mut self) -> PR<Vec<Node>> {
        if self.is_kw("do") {
            self.advance();
        } else if matches!(self.peek().tok, Tok::Newline) || self.is_op(";") {
            self.skip_terms();
        } else {
            return Err(self.unexpected());
        }
        self.loop_depth += 1;
        let body = self.parse_statements(&|t| matches!(t, Tok::Kw("end")));
        self.loop_depth -= 1;
        let body = body?;
        self.expect_kw("end")?;
        Ok(body)
    }

    #[inline(never)]
    fn parse_while(&mut self, until: bool) -> PR<Node> {
        let line = self.line();
        self.enter()?;
        self.advance();
        let r = (|| -> PR<Node> {
            let cond = self.parse_cond()?;
            let body = self.parse_loop_body()?;
            Ok(Node::new(
                Kind::While {
                    cond: Box::new(cond),
                    body,
                    until,
                },
                line,
            ))
        })();
        self.leave();
        r
    }

    #[inline(never)]
    fn parse_for(&mut self) -> PR<Node> {
        let line = self.line();
        self.enter()?;
        self.advance();
        let r = (|| -> PR<Node> {
            let mut vars = Vec::new();
            loop {
                let t = self.advance();
                match t.tok {
                    Tok::Ident(n) => vars.push(self.declare(&n)),
                    _ => return Err(syntax_error("expected a loop variable", t.line)),
                }
                if self.is_op(",") {
                    self.advance();
                    continue;
                }
                break;
            }
            self.expect_kw("in")?;
            let iter = self.parse_cond()?;
            let body = self.parse_loop_body()?;
            Ok(Node::new(
                Kind::For {
                    vars,
                    iter: Box::new(iter),
                    body,
                },
                line,
            ))
        })();
        self.leave();
        r
    }

    #[inline(never)]
    fn parse_case(&mut self) -> PR<Node> {
        let line = self.line();
        self.enter()?;
        self.advance();
        let r = (|| -> PR<Node> {
            let subject = if matches!(self.peek().tok, Tok::Newline) || self.is_op(";") || self.is_kw("when") {
                None
            } else {
                Some(Box::new(self.parse_expr()?))
            };
            self.skip_terms();
            let mut whens = Vec::new();
            while self.is_kw("when") {
                self.advance();
                let mut vals = vec![self.parse_ternary()?];
                while self.is_op(",") {
                    self.advance();
                    self.skip_newlines();
                    vals.push(self.parse_ternary()?);
                }
                self.parse_then()?;
                let enders = |t: &Tok| matches!(t, Tok::Kw("when") | Tok::Kw("else") | Tok::Kw("end"));
                let body = self.parse_statements(&enders)?;
                whens.push((vals, body));
            }
            if whens.is_empty() {
                return Err(self.unexpected());
            }
            let els = if self.is_kw("else") {
                self.advance();
                Some(self.parse_statements(&|t| matches!(t, Tok::Kw("end")))?)
            } else {
                None
            };
            self.expect_kw("end")?;
            Ok(Node::new(Kind::Case { subject, whens, els }, line))
        })();
        self.leave();
        r
    }
}

/// Joins neighbouring literal parts of a string.
fn merge_lits(parts: Vec<StrPart>) -> Vec<StrPart> {
    let mut out: Vec<StrPart> = Vec::new();
    for p in parts {
        match (out.last_mut(), p) {
            (Some(StrPart::Lit(a)), StrPart::Lit(b)) => a.push_str(&b),
            (_, p) => out.push(p),
        }
    }
    if out.is_empty() {
        out.push(StrPart::Lit(String::new()));
    }
    out
}

fn call(recv: Node, name: &str, args: Vec<Node>, line: u32) -> Node {
    Node::new(
        Kind::Call {
            recv: Box::new(recv),
            name: Arc::from(name),
            args: args.into_iter().map(Arg::Pos).collect(),
            block: None,
            safe: false,
        },
        line,
    )
}
