# Copyright 2026 OpenC3, Inc.
# All Rights Reserved.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
# See LICENSE.md for more details.

"""Tree-walking interpreter."""

from __future__ import annotations

import sys
from collections.abc import Callable
from typing import TYPE_CHECKING, Any

from . import nodes as N
from .errors import Error, RubyError
from .values import (
    RClass,
    REnumerator,
    RHash,
    RRange,
    Symbol,
    Value,
    class_name,
    cmp,
    inspect,
    req,
    to_s,
    truthy,
)

if TYPE_CHECKING:
    from .sandbox import Sandbox

MAX_RENDER_DEPTH = 32

MUTATORS: dict[type, frozenset[str]] = {
    list: frozenset(
        "<< push append pop shift unshift prepend insert concat delete delete_at delete_if clear fill []=".split()
    ),
    RHash: frozenset("[]= store delete delete_if clear merge! update default=".split()),
}
MAX_BLOCK_DEPTH = 1024


class BreakEx(Exception):
    def __init__(self, value: Value, tag: object) -> None:
        super().__init__()
        self.value = value
        self.tag = tag


class NextEx(Exception):
    def __init__(self, value: Value) -> None:
        super().__init__()
        self.value = value


class _Buf:
    """Value of a text or ``<%= %>`` statement: the output buffer so far (ERB's ``_erbout``).
    Materialized lazily, only where the value is actually used."""

    __slots__ = ()


BUF = _Buf()


class Scope:
    __slots__ = ("vars", "parent")

    def __init__(self, parent: Scope | None) -> None:
        self.vars: dict[str, Value] = {}
        self.parent = parent


class Block:
    """A template block (closure over the scope it was created in)."""

    __slots__ = ("node", "scope", "interp")

    def __init__(self, node: N.BlockNode, scope: Scope, interp: Interpreter) -> None:
        self.node = node
        self.scope = scope
        self.interp = interp


class NativeBlock:
    """A block implemented by the interpreter itself (enumerator adapters, ``for``)."""

    __slots__ = ("fn",)

    def __init__(self, fn: Callable[[list], Value]) -> None:
        self.fn = fn


class SymBlock:
    """``&:name``: calls method ``name`` on the first yielded value."""

    __slots__ = ("name",)

    def __init__(self, name: str) -> None:
        self.name = name


AnyBlock = Block | NativeBlock | SymBlock | None


class Context:
    """State shared by a render and all of its partials."""

    def __init__(self, sandbox: Sandbox, max_steps: int | None, parse_fn: Callable[..., Any]) -> None:
        self.sandbox = sandbox
        self.max_steps = sys.maxsize if max_steps is None else max_steps
        self.steps = 0
        self.render_depth = 0
        self.block_depth = 0
        self.parse_fn = parse_fn
        self.frozen: dict[int, Value] = {}


class Interpreter:
    def __init__(self, ctx: Context, filename: str) -> None:
        self.ctx = ctx
        self.filename = filename
        self.out: list[str] = []
        self.scope = Scope(None)
        self.line = 1
        self.break_tag: object = None
        self.handlers: dict[type, Callable[[Any], Value]] = {
            N.Text: self.ev_text,
            N.Output: self.ev_output,
            N.Seq: self.ev_seq,
            N.Literal: self.ev_literal,
            N.Str: self.ev_str,
            N.DSym: self.ev_dsym,
            N.ArrayLit: self.ev_array,
            N.HashLit: self.ev_hash,
            N.RangeLit: self.ev_range,
            N.LVar: self.ev_lvar,
            N.LAsgn: self.ev_lasgn,
            N.IndexAsgn: self.ev_index_asgn,
            N.AttrAsgn: self.ev_attr_asgn,
            N.OpAsgn: self.ev_op_asgn,
            N.MultiAsgn: self.ev_multi_asgn,
            N.Call: self.ev_call,
            N.Const: self.ev_const,
            N.And: self.ev_and,
            N.Or: self.ev_or,
            N.Not: self.ev_not,
            N.If: self.ev_if,
            N.While: self.ev_while,
            N.For: self.ev_for,
            N.Case: self.ev_case,
            N.Break: self.ev_break,
            N.Next: self.ev_next,
            N.Shovel: self.ev_shovel,
        }

    # -- entry -----------------------------------------------------------------------------

    def run(self, tree: N.Seq, variables: dict[str, Value]) -> str:
        self.scope.vars.update(variables)
        try:
            self.ev_pass(tree)
        except RubyError as e:
            line = e.line if e.line is not None else self.line
            raise Error(e.type, e.message, self.filename, line) from None
        except RecursionError:
            raise Error("LimitError", "stack level too deep", self.filename, self.line) from None
        except (BreakEx, NextEx):
            raise Error("SyntaxError", "break/next outside of a loop", self.filename, self.line) from None
        except (Error, MemoryError, KeyboardInterrupt):
            raise
        except Exception as e:  # an interpreter bug must not escape as a raw Python exception
            raise Error("RuntimeError", f"internal error: {type(e).__name__}: {e}", self.filename, self.line) from e
        return "".join(self.out)

    # -- evaluation ------------------------------------------------------------------------

    def step(self) -> None:
        ctx = self.ctx
        ctx.steps += 1
        if ctx.steps > ctx.max_steps:
            raise RubyError("LimitError", f"step limit exceeded ({ctx.max_steps})")

    def ev_pass(self, node: N.Node) -> Value:
        """Evaluates a node; a text/output statement value stays as the BUF marker."""
        ctx = self.ctx
        ctx.steps += 1
        if ctx.steps > ctx.max_steps:
            raise RubyError("LimitError", f"step limit exceeded ({ctx.max_steps})", node.line)
        self.line = node.line
        return self.handlers[node.__class__](node)

    def ev(self, node: N.Node) -> Value:
        v = self.ev_pass(node)
        if v is BUF:
            return "".join(self.out)
        return v

    def ev_text(self, node: N.Text) -> Value:
        self.out.append(node.text)
        return BUF

    def ev_output(self, node: N.Output) -> Value:
        v = self.ev(node.expr)
        self.out.append(to_s(v))
        return BUF

    def ev_seq(self, node: N.Seq) -> Value:
        result = None
        for s in node.stmts:
            result = self.ev_pass(s)
        return result

    def ev_literal(self, node: N.Literal) -> Value:
        return node.value

    def ev_str(self, node: N.Str) -> Value:
        out = []
        for p in node.parts:
            if isinstance(p, str):
                out.append(p)
            else:
                out.append(to_s(self.ev(p)))
        return "".join(out)

    def ev_dsym(self, node: N.DSym) -> Value:
        return Symbol(self.ev_str(node.str))

    def ev_array(self, node: N.ArrayLit) -> Value:
        return [self.ev(e) for e in node.elems]

    def ev_hash(self, node: N.HashLit) -> Value:
        h = RHash()
        for k, v in node.pairs:
            key = self.ev(k)
            h.set(key, self.ev(v))
        return h

    def ev_range(self, node: N.RangeLit) -> Value:
        lo = self.ev(node.lo) if node.lo is not None else None
        hi = self.ev(node.hi) if node.hi is not None else None
        if lo is not None and hi is not None and cmp(lo, hi) is None:
            raise RubyError("ArgumentError", "bad value for range", node.line)
        return RRange(lo, hi, node.excl)

    def _scope_at(self, hops: int) -> Scope:
        s = self.scope
        for _ in range(hops):
            assert s.parent is not None
            s = s.parent
        return s

    def ev_lvar(self, node: N.LVar) -> Value:
        return self._scope_at(node.hops).vars.get(node.name)

    def ev_lasgn(self, node: N.LAsgn) -> Value:
        v = self.ev(node.value)
        self._scope_at(node.hops).vars[node.name] = v
        return v

    def ev_index_asgn(self, node: N.IndexAsgn) -> Value:
        recv = self.ev(node.recv)
        args = [self.ev(a) for a in node.args]
        v = self.ev(node.value)
        self.invoke(recv, "[]=", args + [v], None, node.line)
        return v

    def ev_attr_asgn(self, node: N.AttrAsgn) -> Value:
        recv = self.ev(node.recv)
        if node.safe and recv is None:
            return None
        v = self.ev(node.value)
        self.invoke(recv, node.name + "=", [v], None, node.line)
        return v

    def binop(self, a: Value, op: str, b_node: N.Node, line: int) -> Value:
        if op == "||":
            return a if truthy(a) else self.ev(b_node)
        if op == "&&":
            return self.ev(b_node) if truthy(a) else a
        b = self.ev(b_node)
        return self.invoke(a, op, [b], None, line)

    def ev_op_asgn(self, node: N.OpAsgn) -> Value:
        t = node.target
        op = node.op
        if isinstance(t, N.LVar):
            scope = self._scope_at(t.hops)
            cur = scope.vars.get(t.name)
            if (op == "||" and truthy(cur)) or (op == "&&" and not truthy(cur)):
                return cur
            v = self.binop(cur, op, node.value, node.line)
            scope.vars[t.name] = v
            return v
        assert isinstance(t, N.Call) and t.recv is not None
        recv = self.ev(t.recv)
        if t.name == "[]":
            args = [self.ev(a) for a in t.args]
            cur = self.invoke(recv, "[]", args, None, node.line)
            if (op == "||" and truthy(cur)) or (op == "&&" and not truthy(cur)):
                return cur
            v = self.binop(cur, op, node.value, node.line)
            self.invoke(recv, "[]=", args + [v], None, node.line)
            return v
        if t.safe and recv is None:
            return None
        cur = self.invoke(recv, t.name, [], None, node.line)
        if (op == "||" and truthy(cur)) or (op == "&&" and not truthy(cur)):
            return cur
        v = self.binop(cur, op, node.value, node.line)
        self.invoke(recv, t.name + "=", [v], None, node.line)
        return v

    def assign_target(self, t: N.Node, v: Value) -> None:
        if isinstance(t, N.LVar):
            self._scope_at(t.hops).vars[t.name] = v
            return
        assert isinstance(t, N.Call) and t.recv is not None
        recv = self.ev(t.recv)
        if t.name == "[]":
            args = [self.ev(a) for a in t.args]
            self.invoke(recv, "[]=", args + [v], None, t.line)
        else:
            if t.safe and recv is None:
                return
            self.invoke(recv, t.name + "=", [v], None, t.line)

    def ev_multi_asgn(self, node: N.MultiAsgn) -> Value:
        values = [self.ev(v) for v in node.values]
        if len(values) == 1:
            rhs = values[0]
            vals = rhs if type(rhs) is list else [rhs]
        else:
            rhs = values
            vals = values
        for i, t in enumerate(node.targets):
            self.assign_target(t, vals[i] if i < len(vals) else None)
        return rhs

    def ev_shovel(self, node: N.Shovel) -> Value:
        base = self.ev(node.target)
        val = self.ev(node.value)
        if isinstance(base, str):
            from .builtins_str import str_shovel

            self.line = node.line
            try:
                new = str_shovel(base, val)
            except RubyError as e:
                if e.line is None:
                    e.line = node.line
                raise
            root = node.target
            while isinstance(root, N.Shovel):
                root = root.target
            self.assign_target(root, new)
            return new
        return self.invoke(base, "<<", [val], None, node.line)

    def make_block(self, node: N.Call) -> AnyBlock:
        if node.block is not None:
            return Block(node.block, self.scope, self)
        if node.block_sym is not None:
            return SymBlock(node.block_sym)
        return None

    def ev_call(self, node: N.Call) -> Value:
        if node.recv is None:
            return self.call_function(node)
        recv = self.ev(node.recv)
        if node.safe and recv is None:
            return None
        args = [self.ev(a) for a in node.args] if node.args else []
        blk = self.make_block(node)
        return self.invoke(recv, node.name, args, blk, node.line)

    def invoke(self, recv: Value, name: str, args: list, blk: AnyBlock, line: int) -> Value:
        self.line = line
        try:
            return self.call_method(recv, name, args, blk)
        except BreakEx as e:
            if blk is not None and e.tag is blk:
                return e.value
            raise
        except RubyError as e:
            if e.line is None:
                e.line = line
            raise

    def call_function(self, node: N.Call) -> Value:
        from .builtins import FUNCTIONS

        fn = FUNCTIONS.get(node.name)
        if fn is None:
            if not node.args and not node.has_parens and node.block is None and node.block_sym is None:
                raise RubyError(
                    "NameError", f"undefined local variable or method '{node.name}' for main:Object", node.line
                )
            raise RubyError("NoMethodError", f"undefined method '{node.name}' for main:Object", node.line)
        args = [self.ev(a) for a in node.args] if node.args else []
        blk = self.make_block(node)
        self.line = node.line
        try:
            return fn(self, args, blk)
        except BreakEx as e:
            if blk is not None and e.tag is blk:
                return e.value
            raise
        except RubyError as e:
            if e.line is None:
                e.line = node.line
            raise

    def call_method(self, recv: Value, name: str, args: list, blk: AnyBlock) -> Value:
        from .builtins import find_method, no_method

        h = find_method(recv, name)
        if h is None:
            raise no_method(recv, name)
        if self.ctx.frozen and id(recv) in self.ctx.frozen and name in MUTATORS.get(type(recv), ()):
            raise RubyError("FrozenError", f"can't modify frozen {class_name(recv)}: {inspect(recv)}")
        res = h(self, recv, args, blk)
        if type(res) is REnumerator and res.recv is recv and res.meth != name and blk is None:
            res.meth = name  # an alias (collect, each_pair, ...) names its own enumerator
        return res

    def ev_const(self, node: N.Const) -> Value:
        from .builtins import lookup_const

        if node.scope is None:
            return lookup_const(None, node.name, node.line)
        parent = self.ev(node.scope)
        return lookup_const(parent, node.name, node.line)

    def ev_and(self, node: N.And) -> Value:
        left = self.ev(node.left)
        if not truthy(left):
            return left
        return self.ev(node.right)

    def ev_or(self, node: N.Or) -> Value:
        left = self.ev(node.left)
        if truthy(left):
            return left
        return self.ev(node.right)

    def ev_not(self, node: N.Not) -> Value:
        return not truthy(self.ev(node.expr))

    def ev_if(self, node: N.If) -> Value:
        if truthy(self.ev(node.cond)):
            return self.ev_pass(node.then) if node.then is not None else None
        return self.ev_pass(node.else_) if node.else_ is not None else None

    def ev_while(self, node: N.While) -> Value:
        tag = object()
        saved = self.break_tag
        self.break_tag = tag
        try:
            while True:
                c = truthy(self.ev(node.cond))
                if c == node.until:
                    break
                try:
                    self.ev_pass(node.body)
                except NextEx:
                    pass
        except BreakEx as e:
            if e.tag is tag:
                return e.value
            raise
        finally:
            self.break_tag = saved
        return None

    def ev_for(self, node: N.For) -> Value:
        iterable = self.ev(node.iter)
        tag = object()
        targets = node.targets

        def body(vals: list) -> Value:
            if len(targets) == 1:
                v = vals[0] if len(vals) == 1 else (list(vals) if vals else None)
                self._scope_at(targets[0].hops).vars[targets[0].name] = v
            else:
                if len(vals) == 1 and type(vals[0]) is list:
                    vals = vals[0]
                for i, t in enumerate(targets):
                    self._scope_at(t.hops).vars[t.name] = vals[i] if i < len(vals) else None
            saved = self.break_tag
            self.break_tag = tag
            try:
                self.ev_pass(node.body)
            except NextEx:
                pass
            finally:
                self.break_tag = saved
            return None

        try:
            return self.invoke(iterable, "each", [], NativeBlock(body), node.line)
        except BreakEx as e:
            if e.tag is tag:
                return e.value
            raise

    def ev_case(self, node: N.Case) -> Value:
        from .builtins import case_eq

        if node.subject is not None:
            v = self.ev(node.subject)
            for conds, body in node.whens:
                for c in conds:
                    pat = self.ev(c)
                    if case_eq(self, pat, v):
                        return self.ev_pass(body)
        else:
            for conds, body in node.whens:
                for c in conds:
                    if truthy(self.ev(c)):
                        return self.ev_pass(body)
        if node.else_ is not None:
            return self.ev_pass(node.else_)
        return None

    def ev_break(self, node: N.Break) -> Value:
        v = self.ev(node.value) if node.value is not None else None
        raise BreakEx(v, self.break_tag)

    def ev_next(self, node: N.Next) -> Value:
        v = self.ev(node.value) if node.value is not None else None
        raise NextEx(v)

    # -- blocks ----------------------------------------------------------------------------

    def yield_(self, blk: AnyBlock, *vals: Value) -> Value:
        return self.call_block(blk, list(vals))

    def yield_void(self, blk: AnyBlock, *vals: Value) -> None:
        """Yields when the block's value is ignored (each, times, ...)."""
        self.call_block(blk, list(vals), want=False)

    def call_block(self, blk: AnyBlock, vals: list, want: bool = True) -> Value:
        if blk is None:
            raise RubyError("ArgumentError", "no block given (yield)")
        t = type(blk)
        if t is NativeBlock:
            return blk.fn(vals)
        if t is SymBlock:
            if not vals:
                raise RubyError("ArgumentError", "no receiver given")
            return self.call_method(vals[0], blk.name, vals[1:], None)
        assert isinstance(blk, Block)
        return blk.interp.run_block(blk, vals, want)

    def run_block(self, blk: Block, vals: list, want: bool = True) -> Value:
        ctx = self.ctx
        ctx.steps += 1
        if ctx.steps > ctx.max_steps:
            raise RubyError("LimitError", f"step limit exceeded ({ctx.max_steps})")
        node = blk.node
        scope = Scope(blk.scope)
        params = node.params
        np = node.nparams
        if np == 1 and type(params[0]) is str:
            scope.vars[params[0]] = vals[0] if vals else None
        elif np:
            if np > 1 and len(vals) == 1 and type(vals[0]) is list:
                vals = vals[0]
            for i, p in enumerate(params):
                _bind_param(scope.vars, p, vals[i] if i < len(vals) else None)
        saved_scope, saved_tag = self.scope, self.break_tag
        self.scope = scope
        self.break_tag = blk
        ctx.block_depth += 1
        if ctx.block_depth > MAX_BLOCK_DEPTH:
            ctx.block_depth -= 1
            raise RubyError("LimitError", "block nesting too deep")
        try:
            v = self.ev_pass(node.body)
            if v is BUF:
                return "".join(self.out) if want else None
            return v
        except NextEx as e:
            return e.value
        finally:
            ctx.block_depth -= 1
            self.scope, self.break_tag = saved_scope, saved_tag

    def block_arity(self, blk: AnyBlock) -> int:
        if type(blk) is Block:
            return blk.node.nparams
        return -1

    # -- partials --------------------------------------------------------------------------

    def render_partial(self, path: str, locals_: Value) -> str:
        ctx = self.ctx
        if not isinstance(path, str):
            raise RubyError("TypeError", f"no implicit conversion of {class_name(path)} into String")
        variables: dict[str, Value] = {}
        if locals_ is not None:
            if type(locals_) is not RHash:
                raise RubyError("TypeError", f"no implicit conversion of {class_name(locals_)} into Hash")
            for k, v in locals_.items():
                if type(k) is Symbol:
                    variables[k.name] = v
                elif isinstance(k, str):
                    variables[k] = v
                else:
                    raise RubyError("TypeError", f"{class_name(k)} is not a symbol nor a string")
        if ctx.render_depth >= MAX_RENDER_DEPTH:
            raise RubyError("LimitError", f"render nesting too deep (limit {MAX_RENDER_DEPTH})")
        source = ctx.sandbox.read(path)
        tree = ctx.parse_fn(source, path, variables.keys())
        sub = Interpreter(ctx, path)
        ctx.render_depth += 1
        try:
            return sub.run(tree, variables)
        finally:
            ctx.render_depth -= 1


def _bind_param(vars_: dict[str, Value], p: str | list, v: Value) -> None:
    if type(p) is str:
        vars_[p] = v
        return
    sub = v if type(v) is list else [v]
    for j, q in enumerate(p):
        _bind_param(vars_, q, sub[j] if j < len(sub) else None)


def case_eq_basic(pat: Value, v: Value) -> bool:
    if type(pat) is RClass:
        from .values import is_a

        return is_a(v, pat)
    return req(pat, v)
