# Copyright 2026 OpenC3, Inc.
# All Rights Reserved.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
# See LICENSE.md for more details.

"""AST node definitions."""

from __future__ import annotations

from typing import Any


class Node:
    __slots__ = ("line",)

    def __init__(self, line: int) -> None:
        self.line = line


class Text(Node):
    __slots__ = ("text",)

    def __init__(self, line: int, text: str) -> None:
        super().__init__(line)
        self.text = text


class Output(Node):
    __slots__ = ("expr",)

    def __init__(self, line: int, expr: Node) -> None:
        super().__init__(line)
        self.expr = expr


class Seq(Node):
    __slots__ = ("stmts",)

    def __init__(self, line: int, stmts: list[Node]) -> None:
        super().__init__(line)
        self.stmts = stmts


class Literal(Node):
    """nil, true, false, Integer, Float or a plain (non-interpolated) String / Symbol."""

    __slots__ = ("value",)

    def __init__(self, line: int, value: Any) -> None:
        super().__init__(line)
        self.value = value


class Str(Node):
    """Interpolated string: parts are str or Node."""

    __slots__ = ("parts",)

    def __init__(self, line: int, parts: list) -> None:
        super().__init__(line)
        self.parts = parts


class DSym(Node):
    __slots__ = ("str",)

    def __init__(self, line: int, str_: Str) -> None:
        super().__init__(line)
        self.str = str_


class ArrayLit(Node):
    __slots__ = ("elems",)

    def __init__(self, line: int, elems: list[Node]) -> None:
        super().__init__(line)
        self.elems = elems


class HashLit(Node):
    __slots__ = ("pairs",)

    def __init__(self, line: int, pairs: list[tuple[Node, Node]]) -> None:
        super().__init__(line)
        self.pairs = pairs


class RangeLit(Node):
    __slots__ = ("lo", "hi", "excl")

    def __init__(self, line: int, lo: Node | None, hi: Node | None, excl: bool) -> None:
        super().__init__(line)
        self.lo = lo
        self.hi = hi
        self.excl = excl


class LVar(Node):
    __slots__ = ("name", "hops")

    def __init__(self, line: int, name: str, hops: int) -> None:
        super().__init__(line)
        self.name = name
        self.hops = hops


class LAsgn(Node):
    __slots__ = ("name", "hops", "value")

    def __init__(self, line: int, name: str, hops: int, value: Node) -> None:
        super().__init__(line)
        self.name = name
        self.hops = hops
        self.value = value


class IndexAsgn(Node):
    __slots__ = ("recv", "args", "value")

    def __init__(self, line: int, recv: Node, args: list[Node], value: Node) -> None:
        super().__init__(line)
        self.recv = recv
        self.args = args
        self.value = value


class AttrAsgn(Node):
    __slots__ = ("recv", "name", "value", "safe")

    def __init__(self, line: int, recv: Node, name: str, value: Node, safe: bool) -> None:
        super().__init__(line)
        self.recv = recv
        self.name = name
        self.value = value
        self.safe = safe


class OpAsgn(Node):
    """target op= value. target is LVar, Call('[]') or Call(attr)."""

    __slots__ = ("target", "op", "value")

    def __init__(self, line: int, target: Node, op: str, value: Node) -> None:
        super().__init__(line)
        self.target = target
        self.op = op
        self.value = value


class MultiAsgn(Node):
    __slots__ = ("targets", "values")

    def __init__(self, line: int, targets: list[Node], values: list[Node]) -> None:
        super().__init__(line)
        self.targets = targets  # LVar / Call('[]') / Call(attr) used as assignment targets
        self.values = values


class BlockNode(Node):
    """A literal block. params: list of str | list (destructuring)."""

    __slots__ = ("params", "body", "nparams")

    def __init__(self, line: int, params: list, body: Node) -> None:
        super().__init__(line)
        self.params = params
        self.body = body
        self.nparams = len(params)


class Call(Node):
    """Method call. recv None means a bare function call (``format(...)``)."""

    __slots__ = ("recv", "name", "args", "block", "block_sym", "safe", "has_parens")

    def __init__(
        self,
        line: int,
        recv: Node | None,
        name: str,
        args: list[Node] | None = None,
        block: BlockNode | None = None,
        block_sym: str | None = None,
        safe: bool = False,
        has_parens: bool = False,
    ) -> None:
        super().__init__(line)
        self.recv = recv
        self.name = name
        self.args = args or []
        self.block = block
        self.block_sym = block_sym
        self.safe = safe
        self.has_parens = has_parens


class Const(Node):
    __slots__ = ("scope", "name")

    def __init__(self, line: int, scope: Node | None, name: str) -> None:
        super().__init__(line)
        self.scope = scope
        self.name = name


class And(Node):
    __slots__ = ("left", "right")

    def __init__(self, line: int, left: Node, right: Node) -> None:
        super().__init__(line)
        self.left = left
        self.right = right


class Or(Node):
    __slots__ = ("left", "right")

    def __init__(self, line: int, left: Node, right: Node) -> None:
        super().__init__(line)
        self.left = left
        self.right = right


class Not(Node):
    __slots__ = ("expr",)

    def __init__(self, line: int, expr: Node) -> None:
        super().__init__(line)
        self.expr = expr


class If(Node):
    __slots__ = ("cond", "then", "else_")

    def __init__(self, line: int, cond: Node, then: Node | None, else_: Node | None) -> None:
        super().__init__(line)
        self.cond = cond
        self.then = then
        self.else_ = else_


class While(Node):
    __slots__ = ("cond", "body", "until")

    def __init__(self, line: int, cond: Node, body: Node, until: bool) -> None:
        super().__init__(line)
        self.cond = cond
        self.body = body
        self.until = until


class For(Node):
    __slots__ = ("targets", "iter", "body")

    def __init__(self, line: int, targets: list[LVar], iter_: Node, body: Node) -> None:
        super().__init__(line)
        self.targets = targets
        self.iter = iter_
        self.body = body


class Case(Node):
    __slots__ = ("subject", "whens", "else_")

    def __init__(self, line: int, subject: Node | None, whens: list[tuple[list[Node], Node]], else_: Node | None):
        super().__init__(line)
        self.subject = subject
        self.whens = whens
        self.else_ = else_


class Break(Node):
    __slots__ = ("value",)

    def __init__(self, line: int, value: Node | None) -> None:
        super().__init__(line)
        self.value = value


class Next(Node):
    __slots__ = ("value",)

    def __init__(self, line: int, value: Node | None) -> None:
        super().__init__(line)
        self.value = value


class Shovel(Node):
    """``target << value``. For a String target that can be assigned (a local variable or
    ``a[i]``), ERBLite stores the new string back into the target (strings are values)."""

    __slots__ = ("target", "value")

    def __init__(self, line: int, target: Node, value: Node) -> None:
        super().__init__(line)
        self.target = target
        self.value = value
