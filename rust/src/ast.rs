// Copyright 2026 OpenC3, Inc.
// All Rights Reserved.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
// See LICENSE.md for more details.

//! Abstract syntax tree produced by the parser.

use std::sync::Arc;

/// Reference to a local variable resolved at parse time.
#[derive(Debug, Clone)]
pub struct VarRef {
    pub name: Arc<str>,
    /// Index of the scope (0 = template top level, +1 per enclosing block).
    pub depth: usize,
}

#[derive(Debug)]
pub struct Node {
    pub kind: Kind,
    pub line: u32,
}

#[derive(Debug)]
pub enum Arg {
    Pos(Node),
    Pair(Node, Node),
    BlockPass(Arc<str>),
}

#[derive(Debug)]
pub enum Param {
    Var(VarRef),
    Destructure(Vec<Param>),
}

#[derive(Debug)]
pub struct BlockNode {
    pub params: Vec<Param>,
    pub body: Vec<Node>,
    /// True when one yielded Array is spread across the parameters.
    pub splat: bool,
}

/// Assignment target for compound assignment.
#[derive(Debug)]
pub enum Target {
    Var(VarRef),
    Index(Box<Node>, Vec<Arg>),
    Attr(Box<Node>, Arc<str>),
}

#[derive(Debug)]
pub enum Kind {
    Nil,
    True,
    False,
    Int(i64),
    /// An integer literal outside the 64-bit range: raises RangeError when evaluated.
    IntOverflow,
    Float(f64),
    Str(Arc<str>),
    DStr(Vec<Node>),
    Sym(Arc<str>),
    DSym(Vec<Node>),
    Array(Vec<Node>),
    Hash(Vec<(Node, Node)>),
    Range(Option<Box<Node>>, Option<Box<Node>>, bool),
    Text(Arc<str>),
    Output(Box<Node>),
    Var(VarRef),
    Assign(VarRef, Box<Node>),
    /// Identifier that is not a known local: host variable or built-in function.
    IdentCall {
        name: Arc<str>,
        args: Vec<Arg>,
        has_args: bool,
        block: Option<Box<BlockNode>>,
    },
    Call {
        recv: Box<Node>,
        name: Arc<str>,
        args: Vec<Arg>,
        block: Option<Box<BlockNode>>,
        safe: bool,
    },
    Const(Arc<str>),
    ScopedConst(Box<Node>, Arc<str>),
    IndexAssign(Box<Node>, Vec<Arg>, Box<Node>),
    AttrAssign(Box<Node>, Arc<str>, Box<Node>),
    OpAssign(Target, Arc<str>, Box<Node>),
    MultiAssign(Vec<VarRef>, Vec<Node>),
    And(Box<Node>, Box<Node>),
    Or(Box<Node>, Box<Node>),
    Not(Box<Node>),
    If(Box<Node>, Vec<Node>, Vec<Node>),
    While {
        cond: Box<Node>,
        body: Vec<Node>,
        until: bool,
    },
    For {
        vars: Vec<VarRef>,
        iter: Box<Node>,
        body: Vec<Node>,
    },
    Case {
        subject: Option<Box<Node>>,
        whens: Vec<(Vec<Node>, Vec<Node>)>,
        els: Option<Vec<Node>>,
    },
    Break(Option<Box<Node>>),
    Next(Option<Box<Node>>),
    Seq(Vec<Node>),
}

impl Node {
    pub fn new(kind: Kind, line: u32) -> Node {
        Node { kind, line }
    }
}
